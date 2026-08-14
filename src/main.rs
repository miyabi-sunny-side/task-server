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
    let listener = TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "server listening");

    let state = AppState::from_env()?;
    if let Err(err) = task_server::flush_pending(&state.outbox_dir, state.notifier.as_ref()) {
        tracing::warn!("startup outbox flush failed: {err}");
    }

    axum::serve(listener, task_server::app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("server stopped");
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
