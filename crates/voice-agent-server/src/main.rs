use anyhow::Context;
use voice_agent_server::{app::application, config::AppConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let path = std::env::var("VOICE_AGENT_CONFIG").unwrap_or_else(|_| "config.toml".into());
    let config = AppConfig::load(&path).with_context(|| format!("load {path}"))?;
    // Model preparation may use the blocking artifact acquirer; keep it outside Tokio's runtime.
    let startup_config = config.clone();
    let app = tokio::task::spawn_blocking(move || application(startup_config))
        .await
        .context("join local provider startup")?
        .context("initialize local VAD/ASR providers")?;
    let listener = tokio::net::TcpListener::bind(config.server.bind).await?;
    tracing::info!(address = %listener.local_addr()?, "voice protocol server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}
