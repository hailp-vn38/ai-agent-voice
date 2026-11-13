use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountedSummary {
    calls: AtomicUsize,
    fail: bool,
}

impl StatusSummarySource for CountedSummary {
    fn summary(&self) -> SummaryFuture<'_> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let result = (!self.fail).then(|| serde_json::json!({"status":"ready"}));
        Box::pin(async move { result })
    }
}

fn live(fail: bool) -> (Arc<SystemLive>, Arc<CountedSummary>) {
    let source = Arc::new(CountedSummary {
        calls: AtomicUsize::new(0),
        fail,
    });
    let live = Arc::new(SystemLive::new(
        Arc::new(SystemMetrics::new()),
        source.clone(),
    ));
    (live, source)
}

#[tokio::test(start_paused = true)]
async fn resource_samples_replace_the_snapshot_and_none_clears_resources() {
    let metrics = Arc::new(SystemMetrics::new());
    let source = Arc::new(CountedSummary {
        calls: AtomicUsize::new(0),
        fail: false,
    });
    let live = Arc::new(SystemLive::new(metrics.clone(), source));
    let snapshot = live.subscribe();
    let stop = CancellationToken::new();
    let task = live.clone().start(stop.clone());
    tokio::task::yield_now().await;

    metrics.publish_sample_for_test();
    tokio::task::yield_now().await;
    assert!(snapshot.borrow().resources.is_some());

    metrics.clear_for_test();
    tokio::task::yield_now().await;
    assert!(snapshot.borrow().resources.is_none());

    stop.cancel();
    task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn status_is_only_summarized_while_observed_and_is_shared_by_subscribers() {
    let (live, source) = live(false);
    let stop = CancellationToken::new();
    let task = live.clone().start(stop.clone());
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(45)).await;
    tokio::task::yield_now().await;
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);

    let first = live.subscribe();
    tokio::task::yield_now().await;
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    assert!(first.borrow().status.checked_at.is_some());
    assert_eq!(
        first.borrow().status.summary,
        Some(serde_json::json!({"status":"ready"}))
    );

    let second = live.subscribe();
    let third = live.subscribe();
    for _ in 0..3 {
        tokio::time::advance(Duration::from_secs(15)).await;
        tokio::task::yield_now().await;
    }
    assert!((3..=4).contains(&source.calls.load(Ordering::SeqCst)));
    assert_eq!(
        second.borrow().status.checked_at,
        third.borrow().status.checked_at
    );

    drop(first);
    drop(second);
    drop(third);
    for _ in 0..3 {
        tokio::time::advance(Duration::from_secs(15)).await;
        tokio::task::yield_now().await;
    }
    let observed = source.calls.load(Ordering::SeqCst);
    assert!((3..=4).contains(&observed));

    stop.cancel();
    task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn failed_status_has_checked_at_but_no_summary_and_stop_ends_task() {
    let (live, source) = live(true);
    let snapshot = live.subscribe();
    let stop = CancellationToken::new();
    let task = live.start(stop.clone());
    tokio::task::yield_now().await;

    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    assert!(snapshot.borrow().status.checked_at.is_some());
    assert!(snapshot.borrow().status.summary.is_none());
    assert!(snapshot.borrow().resources.is_none());
    assert_eq!(snapshot.borrow().v, 1);

    stop.cancel();
    task.await.unwrap();
}
