use anyhow::Context;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use voice_agent_server::{app::startup_with_shutdown, config::AppConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("install Rustls ring CryptoProvider"))?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let path = std::env::var("VOICE_AGENT_CONFIG").unwrap_or_else(|_| "config.toml".into());
    let config = AppConfig::load(&path).with_context(|| format!("load {path}"))?;
    let shutdown_grace = Duration::from_millis(config.shutdown.grace_ms);
    let cancellation = CancellationToken::new();
    let app = startup_with_shutdown(config.clone(), cancellation.clone())
        .await
        .context("initialize database and local providers")?;
    let listener = tokio::net::TcpListener::bind(config.server.bind).await?;
    tracing::info!(address = %listener.local_addr()?, "voice protocol server listening");
    let server_cancellation = cancellation.clone();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(server_cancellation.cancelled_owned())
            .await
    });
    shutdown_signal().await;
    cancellation.cancel();
    match tokio::time::timeout(shutdown_grace, &mut server).await {
        Ok(result) => result.context("join server shutdown")??,
        Err(_) => {
            tracing::warn!(
                ?shutdown_grace,
                "shutdown grace expired; aborting remaining sessions"
            );
            server.abort();
            let _ = server.await;
        }
    }
    Ok(())
}

#[cfg(unix)]
async fn shutdown_signal() {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
    }
    tracing::info!("shutdown signal received");
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}
