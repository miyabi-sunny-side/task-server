use std::{env, error::Error, net::SocketAddr};

use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

use task_server::AppState;
use task_server::state::DEFAULT_BIND_ADDR;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

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
