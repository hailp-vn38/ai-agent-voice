//! Crash, failure, cancellation and concurrency.

use super::*;
use std::time::Duration;

const AFTER_COMMIT: [FailPoint; 4] = [
    FailPoint::Index,
    FailPoint::Publish,
    FailPoint::Summary,
    FailPoint::Cleanup,
];

#[tokio::test]
async fn forget_removes_the_fact_from_recall_context_and_summary_file() {
    let f = fixture();
    let kept = f.remember("Giờ mở cửa", "Mở cửa lúc tám giờ").await;
    let gone = f.remember("Mã cổng", "Mã cổng là bảy bảy").await;
    let handle = f.core.context(&f.scope).await;
    assert!(handle.current().rows.join("\n").contains("Mã cổng"));

    let receipt = f.forget(&gone).await.unwrap();

    assert!(!receipt.cleanup_pending);
    assert_eq!(receipt.action, ReceiptAction::Forget);
    assert!(f.recall("cổng").await.is_empty());
    assert!(!handle.current().rows.join("\n").contains("Mã cổng"));
    assert!(!f.summary_file().contains("Mã cổng"));
    assert_eq!(f.fact_paths().len(), 1);
    assert_eq!(f.only_fact().await.id, kept.fact_id.unwrap());
}

#[tokio::test]
async fn forget_never_comes_back_after_a_crash_at_any_step_past_the_commit_point() {
    for point in AFTER_COMMIT {
        let f = fixture();
        let gone = f.remember("Mã cổng", "Mã cổng là bảy bảy").await;
        f.remember("Giờ mở cửa", "Mở cửa lúc tám giờ").await;
        f.arm(point, FailAction::Crash);
        f.forget(&gone)
            .await
            .expect_err("the crash stops the write");

        let restarted = f.reopen();
        // Before any convergence, every read already honours the tombstone.
        assert!(
            recall_on(&restarted, &f.scope, "cổng").await.is_empty(),
            "{point:?}"
        );
        assert!(
            !context_text(&restarted, &f.scope).await.contains("Mã cổng"),
            "{point:?}"
        );

        let report = restarted.reconcile(&f.scope).await.unwrap();
        assert_eq!(report.facts, 1, "{point:?}");

        assert!(
            recall_on(&restarted, &f.scope, "cổng").await.is_empty(),
            "{point:?}"
        );
        assert!(
            !context_text(&restarted, &f.scope).await.contains("Mã cổng"),
            "{point:?}"
        );
        assert!(!f.summary_file().contains("Mã cổng"), "{point:?}");
        assert_eq!(f.fact_paths().len(), 1, "{point:?}");
        assert!(
            context_text(&restarted, &f.scope)
                .await
                .contains("Giờ mở cửa")
        );
    }
}

#[tokio::test]
async fn forget_crash_before_the_commit_point_leaves_the_fact_in_place() {
    let f = fixture();
    let fact = f.remember("Mã cổng", "Mã cổng là bảy bảy").await;
    f.arm(FailPoint::Commit, FailAction::Crash);
    f.forget(&fact)
        .await
        .expect_err("the crash stops the write");

    let restarted = f.reopen();
    restarted.reconcile(&f.scope).await.unwrap();
    assert_eq!(recall_on(&restarted, &f.scope, "cổng").await.len(), 1);
    assert_eq!(f.fact_paths().len(), 1);
}

#[tokio::test]
async fn failure_after_the_commit_point_returns_ok_with_cleanup_pending() {
    for point in AFTER_COMMIT {
        let f = fixture();
        let gone = f.remember("Mã cổng", "Mã cổng là bảy bảy").await;
        let handle = f.core.context(&f.scope).await;
        f.arm(point, FailAction::Error);

        let receipt = f.forget(&gone).await.expect("past the commit point");

        assert!(receipt.cleanup_pending, "{point:?}");
        assert!(f.recall("cổng").await.is_empty(), "{point:?}");
        assert!(
            !handle.current().rows.join("\n").contains("Mã cổng"),
            "{point:?}"
        );

        f.core.reconcile(&f.scope).await.unwrap();
        assert!(f.fact_paths().is_empty(), "{point:?}");
        assert!(!f.summary_file().contains("Mã cổng"), "{point:?}");
    }
}

#[tokio::test]
async fn failure_after_the_fact_is_written_still_returns_ok_for_remember_and_correct() {
    for point in [FailPoint::Index, FailPoint::Publish, FailPoint::Summary] {
        let f = fixture();
        f.arm(point, FailAction::Error);
        let first = f
            .core
            .apply(
                &f.scope,
                MemoryActor::Admin,
                remember_change("Giờ mở cửa", "Mở cửa lúc tám giờ"),
            )
            .await
            .expect("past the commit point");
        assert!(first.cleanup_pending, "{point:?}");
        assert_eq!(f.recall("tám").await.len(), 1, "{point:?}");

        f.arm(point, FailAction::Error);
        let second = f
            .core
            .apply(
                &f.scope,
                MemoryActor::Admin,
                correct_change(&first, "Mở cửa lúc chín giờ"),
            )
            .await
            .expect("past the commit point");
        assert!(second.cleanup_pending, "{point:?}");
        assert_eq!(f.recall("chín").await.len(), 1, "{point:?}");
    }
}

#[tokio::test]
async fn failure_before_the_commit_point_is_unavailable_and_changes_nothing() {
    let f = fixture();
    let fact = f.remember("Giờ mở cửa", "Mở cửa lúc tám giờ").await;
    let before = fs::read(f.the_fact_path()).unwrap();
    for change in [
        remember_change("Khác", "Nội dung khác"),
        correct_change(&fact, "Mở cửa lúc chín giờ"),
        MemoryChange::Discard {
            id: fact.fact_id.clone().unwrap(),
            expected_revision: 1,
        },
        MemoryChange::Forget {
            id: fact.fact_id.clone().unwrap(),
            expected_revision: 1,
        },
        MemoryChange::SetPinned {
            id: fact.fact_id.clone().unwrap(),
            pinned: true,
        },
    ] {
        f.arm(FailPoint::Commit, FailAction::Error);
        let error = f
            .core
            .apply(&f.scope, MemoryActor::Admin, change)
            .await
            .expect_err("before the commit point");
        assert_eq!(error.code(), "memory_unavailable");
        assert_eq!(fs::read(f.the_fact_path()).unwrap(), before);
    }
    assert_eq!(f.tombstones_file(), "");
}

#[tokio::test]
async fn crash_after_writing_a_fact_converges_on_reconcile() {
    for point in [FailPoint::Index, FailPoint::Publish, FailPoint::Summary] {
        let f = fixture();
        f.remember("Giờ mở cửa", "Mở cửa lúc tám giờ").await;
        assert_eq!(f.recall("tám").await.len(), 1); // The index now exists and would go stale.
        f.arm(point, FailAction::Crash);
        f.core
            .apply(
                &f.scope,
                MemoryActor::Admin,
                remember_change("Mã cổng", "Mã cổng bảy bảy"),
            )
            .await
            .expect_err("the crash stops the write");

        let restarted = f.reopen();
        restarted.reconcile(&f.scope).await.unwrap();

        assert_eq!(
            recall_on(&restarted, &f.scope, "cổng").await.len(),
            1,
            "{point:?}"
        );
        assert!(context_text(&restarted, &f.scope).await.contains("Mã cổng"));
        assert!(f.summary_file().contains("Mã cổng"), "{point:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writes_on_one_scope_lose_no_update() {
    let f = fixture();
    let target = f.remember("Đích", "Thân ban đầu").await;
    let id = target.fact_id.clone().unwrap();
    let mut tasks = tokio::task::JoinSet::new();
    for i in 0..16 {
        let (core, scope) = (f.core.clone(), f.scope.clone());
        tasks.spawn(async move {
            core.apply(
                &scope,
                MemoryActor::Admin,
                remember_change(&format!("Mục {i}"), &format!("nội dung số {i} riêng")),
            )
            .await
            .unwrap();
        });
    }
    for i in 0..8 {
        let (core, scope, id) = (f.core.clone(), f.scope.clone(), id.clone());
        tasks.spawn(async move {
            loop {
                let current = core
                    .inspect(
                        &scope,
                        InspectFilter {
                            id: Some(id.clone()),
                        },
                    )
                    .await
                    .unwrap()
                    .facts[0]
                    .revision;
                let result = core
                    .apply(
                        &scope,
                        MemoryActor::Admin,
                        MemoryChange::Correct {
                            id: id.clone(),
                            expected_revision: current,
                            title: None,
                            hook: None,
                            body: format!("thân đã sửa {i}"),
                        },
                    )
                    .await;
                match result {
                    Ok(_) => break,
                    Err(MemoryError::RevisionConflict) => continue,
                    Err(error) => panic!("{error}"),
                }
            }
        });
    }
    while let Some(joined) = tasks.join_next().await {
        joined.unwrap();
    }

    let overview = f
        .core
        .inspect(&f.scope, InspectFilter::default())
        .await
        .unwrap();
    assert_eq!(overview.facts.len(), 17);
    assert_eq!(f.fact_paths().len(), 17);
    let target = overview.facts.iter().find(|fact| fact.id == id).unwrap();
    assert_eq!(target.revision, 9, "every Correct bumped the revision once");
    assert_eq!(f.core.context(&f.scope).await.current().rows.len(), 17);
    assert_eq!(f.summary_file().matches("- **").count(), 17);
}

#[tokio::test]
async fn dropping_the_apply_future_midway_leaves_the_write_sequence_running() {
    let f = fixture();
    assert!(f.recall("x").await.is_empty());
    f.remember("Có trước", "Có từ trước").await;
    assert_eq!(f.recall("trước").await.len(), 1);
    let handle = f.core.context(&f.scope).await;

    f.arm(
        FailPoint::Index,
        FailAction::Stall(Duration::from_millis(100)),
    );
    let dropped = tokio::time::timeout(
        Duration::from_millis(10),
        f.core.apply(
            &f.scope,
            MemoryActor::Admin,
            remember_change("Mã cổng", "Mã cổng bảy bảy"),
        ),
    )
    .await;
    assert!(
        dropped.is_err(),
        "the caller gave up while the write was stalled"
    );

    eventually("the summary file catches up", async || {
        f.summary_file().contains("Mã cổng")
    })
    .await;
    assert_eq!(f.recall("cổng").await.len(), 1);
    assert!(handle.current().rows.join("\n").contains("Mã cổng"));
    assert_eq!(f.fact_paths().len(), 2);
}

#[tokio::test]
async fn a_writing_failure_in_the_index_does_not_fail_the_write_and_recall_recovers() {
    let f = fixture();
    f.remember("Có trước", "Có từ trước").await;
    assert_eq!(f.recall("trước").await.len(), 1);
    fs::write(f.index_path(), b"this is not a database").unwrap();

    let receipt = f.remember("Mã cổng", "Mã cổng bảy bảy").await;

    assert!(receipt.cleanup_pending);
    assert_eq!(f.recall("cổng").await.len(), 1);
    assert_eq!(f.recall("trước").await.len(), 1);
}

#[tokio::test]
async fn first_context_load_past_its_deadline_returns_empty_then_fills_in() {
    let f = fixture();
    f.remember("Có trước", "Có từ trước").await;
    let restarted = f.reopen();
    // A write stalled while holding the writer keeps the first load waiting past 150 ms.
    *restarted.inner.fail_point.lock().unwrap() = Some((
        FailPoint::Index,
        FailAction::Stall(Duration::from_millis(600)),
    ));
    let writer = {
        let (core, scope) = (restarted.clone(), f.scope.clone());
        tokio::spawn(async move {
            core.apply(
                &scope,
                MemoryActor::Admin,
                remember_change("Mới", "Nội dung mới"),
            )
            .await
            .unwrap();
        })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;

    let started = std::time::Instant::now();
    let handle = restarted.context(&f.scope).await;

    assert!(
        started.elapsed() < Duration::from_millis(500),
        "the deadline bounds admission"
    );
    assert!(handle.current().rows.is_empty());
    writer.await.unwrap();
    eventually("the handle fills in", async || {
        let text = handle.current().rows.join("\n");
        text.contains("Có trước") && text.contains("Mới")
    })
    .await;
}

#[tokio::test]
async fn an_unreadable_tombstone_file_fails_closed() {
    let f = fixture();
    let fact = f.remember("Mã cổng", "Mã cổng bảy bảy").await;
    assert_eq!(f.recall("cổng").await.len(), 1);
    fs::write(f.device_dir().join("tombstones.json"), b"{broken").unwrap();

    let restarted = f.reopen();
    let error = restarted
        .recall(
            &f.scope,
            RecallQuery {
                text: "cổng".into(),
                limit: 5,
            },
        )
        .await
        .expect_err("cannot prove the fact was not forgotten");
    assert_eq!(error.code(), "memory_unavailable");
    assert!(restarted.context(&f.scope).await.current().rows.is_empty());
    let error = restarted
        .apply(
            &f.scope,
            MemoryActor::Admin,
            MemoryChange::Forget {
                id: fact.fact_id.unwrap(),
                expected_revision: 1,
            },
        )
        .await
        .expect_err("must not overwrite tombstones it cannot read");
    assert_eq!(error.code(), "memory_unavailable");
    assert_eq!(
        fs::read(f.device_dir().join("tombstones.json")).unwrap(),
        b"{broken"
    );
}
