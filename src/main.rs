use std::{env, error::Error, net::SocketAddr};

use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use task_server::db::Db;
use task_server::import::{ImportSources, import_markdown};
use task_server::scan::{self, Catalogue};
use task_server::state::{DEFAULT_BIND_ADDR, DEFAULT_DB_PATH};
use task_server::{AppState, SystemClock};

const USAGE: &str = "usage: task-server [import-markdown --live <DIR> [--archive <DIR>]]";

/// No arguments is the server. The one subcommand is the markdown import, which
/// is a migration an operator runs by hand, so it prints a summary and exits
/// rather than binding a socket.
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => serve().await,
        Some("import-markdown") => run_import(&args[1..]),
        Some(other) => refuse(&format!("unknown subcommand '{other}'")),
    }
}

/// Print a usage refusal and exit non-zero. Returning the message as an error
/// would render it through `Debug`, which escapes the line break the usage
/// line needs.
fn refuse(message: &str) -> ! {
    eprintln!("{message}\n{USAGE}");
    std::process::exit(1)
}

/// Import the markdown queue into the database at `APP_DB_PATH`. A refusal is
/// printed whole — every file and every reason — and exits non-zero, because
/// nothing was written and the operator has one list to work from.
fn run_import(args: &[String]) -> Result<(), Box<dyn Error>> {
    let sources = ImportSources::from_args(args).unwrap_or_else(|error| refuse(&error.to_string()));
    let db = Db::open(env::var("APP_DB_PATH").unwrap_or_else(|_| DEFAULT_DB_PATH.to_owned()))?;
    match import_markdown(&db, &sources, &SystemClock) {
        Ok(summary) => {
            print!("{summary}");
            Ok(())
        }
        Err(error) => {
            eprint!("{error}");
            std::process::exit(1)
        }
    }
}

async fn serve() -> Result<(), Box<dyn Error>> {
    let bind_addr = bind_addr_from_env()?;
    // Read before the database is opened, so a refused configuration never
    // leaves a database file behind.
    let catalogue = scan::source_from_vars(|key| env::var(key).ok())?;
    // Fail before the socket exists: a listener that logs "listening" and then
    // exits over an unreadable project tree is worse than never binding at all.
    let state = AppState::from_env()?;
    derive_catalogue(&state, &catalogue)?;
    // The haystack keeps every row; only the two output tails age out. Once per
    // start is enough — the sweep is about disk, not about freshness.
    let blanked =
        task_server::runs::prune_tails(&state.db, state.clock.now(), state.runs_retention_days)?;
    info!(
        blanked,
        retention_days = state.runs_retention_days,
        "run tails swept"
    );

    let listener = TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "server listening");

    axum::serve(listener, task_server::app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("server stopped");
    Ok(())
}

/// Make the catalogue equal the project tree, when one is configured.
///
/// With no tree the catalogue is curated over the API alone and nothing is
/// touched. With one, the walk is fail-closed — a root that cannot be read stops
/// the startup rather than reporting an empty tree, which would read as every
/// product having been deleted at once.
///
/// The log is the operator's record of the drift that was closed: what came and
/// went, what was left alone, and for each product that left the tree, how many
/// tasks still name it.
fn derive_catalogue(state: &AppState, catalogue: &Catalogue) -> Result<(), Box<dyn Error>> {
    let Catalogue::Derived(root) = catalogue else {
        return Ok(());
    };
    let scanned = scan::scan(root)?;
    for skipped in &scanned.skipped {
        info!(
            entry = %skipped.name,
            reason = skipped.reason.as_str(),
            "project skipped"
        );
    }
    let report = task_server::product::reconcile(&state.db, &scanned.products, state.clock.now())?;
    for archived in &report.archived {
        warn!(
            id = %archived.id,
            tasks = archived.tasks,
            "product left the project tree and was archived: it answers history but takes no new work"
        );
    }
    for id in &report.unarchived {
        info!(id = %id, "product came back to the project tree and was unarchived");
    }
    if report.skipped_archive_all {
        warn!(
            root = %root.display(),
            "the walk found no products, so none were archived: check that the project tree is there"
        );
    }
    info!(
        root = %root.display(),
        inserted = report.inserted,
        updated = report.updated,
        unchanged = report.unchanged,
        archived = report.archived.len(),
        unarchived = report.unarchived.len(),
        skipped = ?scanned.skipped_by_reason(),
        "catalogue derived from the project tree"
    );
    Ok(())
}

fn bind_addr_from_env() -> Result<SocketAddr, Box<dyn Error>> {
    env::var("APP_BIND_ADDR")
        .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned())
        .parse()
        .map_err(Into::into)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    info!("shutdown signal received");
}
