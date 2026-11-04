use std::time::Duration;

use super::harness::*;

/// The gate refuses new connections before any session is asked anything, so a request that arrives
/// after shutdown began is refused rather than admitted and immediately closed.
#[tokio::test]
async fn a_closed_gate_refuses_a_new_voice_connection() {
    let voice = start(config(database_url()), Duration::from_secs(15)).await;
    seed(&voice.database().await).await;
    assert_eq!(
        connect_async(request(&voice.base, "device"))
            .await
            .unwrap()
            .1
            .status(),
        StatusCode::SWITCHING_PROTOCOLS,
        "the boundary admits while the process is working"
    );

    // Close the gate without draining anything: this is the state shutdown reaches first.
    assert!(voice.lifecycle.begin_shutdown());

    let refusal = connect_async(request(&voice.base, "device"))
        .await
        .unwrap_err();
    assert_eq!(
        rejected_status(refusal),
        StatusCode::SERVICE_UNAVAILABLE,
        "a closed gate refuses the connection at the boundary"
    );
    assert_eq!(voice.status("/health").await, StatusCode::OK);
}

/// The gate is application-owned, so admission is refused even for a caller that never went near
/// the WebSocket boundary.  This is what "must not depend on a SessionActor observing shutdown"
/// means in practice: the decision belongs to the process, not to whoever happens to call it.
#[tokio::test]
async fn a_closed_gate_refuses_database_admission_regardless_of_the_caller() {
    let voice = start(config(database_url()), Duration::from_secs(15)).await;
    seed(&voice.database().await).await;
    assert!(
        voice.state.resolve_session_profile("device").await.is_ok(),
        "the boundary admits while the process is working"
    );

    voice.lifecycle.begin_shutdown();

    assert_eq!(
        voice.state.resolve_session_profile("device").await.err(),
        Some(SessionProfileAdmissionError::ShuttingDown),
        "no caller can start a database admission after the gate closed"
    );
}
