use std::sync::Arc;

use hi_telos::{
    agent::AgentRuntime,
    config, orchestrator,
    server::{self, ServerState},
    state::AppContext,
};
use tracing::error;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    config::init_tracing();
    let config = config::AppConfig::load()?;
    let agent_runtime = AgentRuntime::from_app_config(&config)?;
    let ctx = AppContext::new(config, Arc::new(agent_runtime));

    let (orchestrator_handle, orchestrator_task) = orchestrator::spawn(ctx.clone());

    let server_state = ServerState::new(ctx.clone(), orchestrator_handle.clone());
    let server_task = tokio::spawn(async move {
        if let Err(err) = server::serve(server_state).await {
            error!(error = ?err, "server error");
        }
    });

    tokio::signal::ctrl_c().await?;
    ctx.request_shutdown();

    let _ = server_task.await;

    if let Err(err) = orchestrator_task.await {
        error!(error = ?err, "orchestrator task join error");
    }

    Ok(())
}
