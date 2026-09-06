mod logging;
mod port;

use std::{env, error::Error};
use task_server::AppState;
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    logging::init();
    if env::args().len() > 1 {
        return Err("use bin/task-data for migration and backup".into());
    }
    let address = std::net::SocketAddr::from(([0, 0, 0, 0], port::from_env()?));
    let state = AppState::from_env()?;
    task_server::task::sweep(&state)?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address,data_dir=%state.store.root().display(),"listening");
    axum::serve(listener, task_server::app(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
