use anyhow::Context;
use voice_agent_server::{
    app::{new_lifecycle, startup_with_lifecycle},
    config::AppConfig,
    lifecycle::CONTROLLED_CLOSE_SETTLE,
};

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
    // Built before startup so the listener, every admitted Voice Session and the shutdown sequence
    // all hold the same admission gate and drain registry. Building it afterwards would leave the
    // process briefly running with two lifecycles and no shared decision between them.
    let lifecycle = new_lifecycle(&config);
    let app = startup_with_lifecycle(config.clone(), lifecycle.clone())
        .await
        .context("initialize database and local providers")?;
    let listener = tokio::net::TcpListener::bind(config.server.bind).await?;
    tracing::info!(address = %listener.local_addr()?, "voice protocol server listening");
    let listening = lifecycle.listening().clone();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, app).with_graceful_shutdown(listening.cancelled_owned()).await
    });
    shutdown_signal().await;
    // The ordered shutdown: close the admission gate so nothing new starts, drain the sessions this
    // process already admitted, controlled-close whatever is still open at the deadline, and flush
    // the archival writer inside that same deadline.
    let report = lifecycle.shutdown().await;
    tracing::info!(
        event = "voice_session_drain_completed",
        outcome = ?report.outcome,
        controlled_closes = report.controlled_closes,
        history_flushed = report.history_flushed,
        "Voice Sessions drained"
    );
    // Every session has been asked to close, so this only waits for the closes to land. It is
    // bounded rather than open-ended, and it is the last thing that may end a process abruptly:
    // a session that has already stopped answering its controlled close is not something a longer
    // wait fixes.
    match tokio::time::timeout(CONTROLLED_CLOSE_SETTLE, &mut server).await {
        Ok(result) => result.context("join server shutdown")??,
        Err(_) => {
            tracing::warn!(
                ?CONTROLLED_CLOSE_SETTLE,
                "Voice Sessions did not finish their controlled close in time; abandoning them"
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
