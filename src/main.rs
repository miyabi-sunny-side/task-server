use std::{env, error::Error, net::SocketAddr};

use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

use task_server::db::Db;
use task_server::import::{ImportSources, import_markdown};
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
    // Fail before the socket exists: a listener that logs "listening" and then
    // exits over a bad seed file is worse than never binding at all.
    let state = AppState::from_env()?;
    seed_products(&state)?;

    let listener = TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "server listening");

    axum::serve(listener, task_server::app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("server stopped");
    Ok(())
}

/// Upsert the roster at `APP_PRODUCTS_SEED`, if one is configured. Unset means
/// the catalogue is curated over the API alone; set and unusable is fatal, so a
/// typo in the path never boots a server with an empty catalogue.
fn seed_products(state: &AppState) -> Result<(), Box<dyn Error>> {
    let Some(path) = env::var("APP_PRODUCTS_SEED").ok().filter(|p| !p.is_empty()) else {
        return Ok(());
    };
    let seeded = task_server::product::seed_from_path(&state.db, &path, state.clock.now())?;
    info!(seeded, path, "products seeded");
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
