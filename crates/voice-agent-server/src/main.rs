use anyhow::Context;
use voice_agent_server::{app::router, config::AppConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let path = std::env::var("VOICE_AGENT_CONFIG").unwrap_or_else(|_| "config.toml".into());
    let config = AppConfig::load(&path).with_context(|| format!("load {path}"))?;
    let listener = tokio::net::TcpListener::bind(config.server.bind).await?;
    tracing::info!(address = %listener.local_addr()?, "voice protocol server listening");
    axum::serve(listener, router(config))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}
