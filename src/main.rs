use std::{env, error::Error};
use task_server::{AppState, state::DEFAULT_BIND_ADDR};
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    if env::args().len() > 1 {
        return Err("use bin/task-data for migration and backup".into());
    }
    let state = AppState::from_env()?;
    if state.projects_dir.is_some() {
        task_server::product::rescan(&state)?;
    }
    task_server::task::sweep(&state)?;
    let address = env::var("APP_BIND_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.into());
    let listener = tokio::net::TcpListener::bind(&address).await?;
    tracing::info!(%address,data_dir=%state.store.root().display(),"listening");
    axum::serve(listener, task_server::app(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
