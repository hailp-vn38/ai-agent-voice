use std::sync::Arc;
use std::time::Duration;

use super::harness::*;

/// A session that finishes on its own drains the process without waiting out the grace deadline.
/// Shutdown returning promptly is the property; a fixed wait would satisfy the deadline and fail
/// this.
#[tokio::test]
async fn a_session_that_finishes_on_its_own_drains_before_the_deadline() {
    let voice = start(config(database_url()), Duration::from_secs(30)).await;
    seed(&voice.database().await).await;
    let (mut socket, _) = connect_async(request(&voice.base, "device")).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::Text(_)
    ));
    // The client goes away on its own terms, which is the ordinary case: the connection ends and
    // the server finishes the session without anybody closing the gate or signalling a deadline.
    drop(socket);
    await_drain(voice.lifecycle.drain().as_ref()).await;

    let started = std::time::Instant::now();
    let report = shutdown_within(Arc::clone(&voice.lifecycle), Duration::from_secs(10)).await;
    assert_eq!(report.outcome, DrainOutcome::Drained);
    assert_eq!(report.controlled_closes, 0);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a completed drain returns on completion, not on the deadline: {:?}",
        started.elapsed()
    );
}

/// A session still open at the grace deadline is issued a controlled close — a protocol close the
/// session performs itself — rather than aborted.  The client sees the normal going-away code, which
/// is the observable difference between the two mechanisms.
#[tokio::test]
async fn a_session_open_at_the_deadline_is_controlled_closed_not_aborted() {
    let voice = start(config(database_url()), Duration::from_millis(50)).await;
    seed(&voice.database().await).await;
    let (mut socket, _) = connect_async(request(&voice.base, "device")).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::Text(_)
    ));
    // The session stays connected and idle across the deadline.

    let started = std::time::Instant::now();
    let lifecycle = Arc::clone(&voice.lifecycle);
    let shutdown = tokio::spawn(async move { lifecycle.shutdown().await });

    let close = timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("the controlled close arrives inside the settle window")
        .expect("the connection is still open")
        .expect("the frame is readable");
    assert!(
        matches!(close, Message::Close(Some(frame)) if u16::from(frame.code) == 1001),
        "a session closed by the drain performs its own protocol close, not a task abort"
    );

    let report = shutdown.await.unwrap();
    assert_eq!(
        report.outcome,
        DrainOutcome::DeadlineReached { remaining: 1 }
    );
    assert_eq!(report.controlled_closes, 1);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the drain stops at the grace deadline instead of waiting the session out: {:?}",
        started.elapsed()
    );
}

/// The deadline is one shared deadline.  A process with a session it cannot close still finishes
/// shutdown in bounded time, and the archival flush gets whatever the drain did not — never a
/// second grace period.
#[tokio::test(start_paused = true)]
async fn shutdown_is_bounded_even_when_a_session_never_answers() {
    // No database and no socket: this test exercises the deadline itself, and a paused clock is
    // only sound when nothing in the process is waiting on real I/O.
    let voice = start_without_database(Duration::from_millis(50)).await;
    // Registered but never connected: a session the drain can signal and nothing will ever answer.
    let silent = voice.state.register_session();

    let grace = voice.lifecycle.grace();
    let lifecycle = Arc::clone(&voice.lifecycle);
    let mut shutdown = Box::pin(lifecycle.shutdown());
    // Nothing has moved the clock, so a drain that closed early would already be finished.
    assert!(
        timeout(Duration::ZERO, shutdown.as_mut()).await.is_err(),
        "the drain waits for the sessions it was given rather than closing them at once"
    );
    tokio::time::advance(grace).await;
    let report = shutdown.await;

    assert_eq!(
        report.outcome,
        DrainOutcome::DeadlineReached { remaining: 1 }
    );
    assert_eq!(report.controlled_closes, 1);
    assert!(voice.lifecycle.stopping().is_cancelled());
    drop(silent);
}

/// The archival writer is best-effort inside the same deadline, and it is never what decides how
/// long shutdown takes.  A writer with nothing left to do is flushed on the first read; one that
/// cannot settle is reported as unflushed and the shutdown still returns.
#[tokio::test]
async fn the_history_flush_never_decides_how_long_shutdown_takes() {
    let mut capture_on = config(database_url());
    capture_on.database.history = DatabaseHistoryConfig {
        enabled: true,
        ..DatabaseHistoryConfig::default()
    };
    let voice = start(capture_on, Duration::from_millis(50)).await;
    seed(&voice.database().await).await;
    let (mut socket, _) = connect_async(request(&voice.base, "device")).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::Text(_)
    ));

    let started = std::time::Instant::now();
    let report = shutdown_within(Arc::clone(&voice.lifecycle), Duration::from_secs(5)).await;
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the archive is best-effort and cannot hold the process open: {:?}",
        started.elapsed()
    );
    // Whether the archive settled is reported, not awaited, so both answers are acceptable here and
    // the property under test is the bound above.
    let _ = report.history_flushed;
    assert!(report.outcome != DrainOutcome::Drained || report.controlled_closes == 0);
    let _ = socket;
}

/// A deployment with the archive off has nothing to flush and reports so, rather than waiting.
#[tokio::test]
async fn a_capture_off_process_reports_nothing_left_to_flush() {
    let voice = start(config(database_url()), Duration::from_millis(50)).await;
    seed(&voice.database().await).await;
    let (mut socket, _) = connect_async(request(&voice.base, "device")).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::Text(_)
    ));

    let report = shutdown_within(Arc::clone(&voice.lifecycle), Duration::from_secs(5)).await;
    assert!(
        report.history_flushed,
        "no writer means nothing unsettled, which is a settled archive"
    );
}

/// One process, several sessions: the drain closes exactly the ones still open and leaves the
/// finished ones alone.  This is what a registry of registered completion handles buys over a
/// broadcast — shutdown can count and address, not merely signal everyone.
#[tokio::test]
async fn the_drain_counts_and_closes_exactly_the_sessions_still_open() {
    // A short grace: this test is about which sessions are counted and signalled, not about how
    // long the process is willing to wait for them.
    let voice = start(config(database_url()), Duration::from_millis(50)).await;
    seed(&voice.database().await).await;
    let finished = voice.state.register_session();
    let open = voice.state.register_session();
    assert_eq!(voice.lifecycle.drain().active(), 2);

    drop(finished);

    let lifecycle = Arc::clone(&voice.lifecycle);
    let report = lifecycle.shutdown().await;
    assert_eq!(
        report.outcome,
        DrainOutcome::DeadlineReached { remaining: 1 }
    );
    assert_eq!(
        report.controlled_closes, 1,
        "only the session still registered is asked to close"
    );
    assert!(
        open.close_signal().is_cancelled(),
        "and it is the one that was signalled"
    );
}

/// The deadline is the one the deployment configured, read from the same validated field the
/// production path reads.  A process configured for a long grace really does wait that long, which
/// is the only way to tell "bounded" from "immediately closed".
#[tokio::test(start_paused = true)]
async fn the_grace_deadline_comes_from_the_deployment_configuration() {
    let mut app_config = config(database_url());
    app_config.shutdown = ShutdownConfig { grace_ms: 1_500 };
    // Built exactly as production builds it: from the configuration's own grace.
    let lifecycle = RuntimeLifecycle::from_config(&app_config);
    assert_eq!(lifecycle.grace(), Duration::from_millis(1_500));

    let silent = lifecycle.drain().register();
    let mut shutdown = Box::pin(lifecycle.shutdown());
    // Nothing has moved the clock. A drain that ignored the configured deadline would be finished
    // already, which is exactly the "immediately closed" behaviour this test exists to rule out.
    assert!(
        timeout(Duration::ZERO, shutdown.as_mut()).await.is_err(),
        "a 1.5s grace means the drain is still waiting with no time elapsed"
    );
    tokio::time::advance(Duration::from_millis(1_500)).await;

    assert_eq!(
        shutdown.await.outcome,
        DrainOutcome::DeadlineReached { remaining: 1 },
        "and at the configured deadline the open session is closed rather than waited out"
    );
    drop(silent);
}
