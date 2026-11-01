//! The semantics of each `MemoryChange`.

use super::*;

#[tokio::test]
async fn discard_removes_the_fact_without_a_tombstone_so_it_can_be_saved_again() {
    let f = fixture();
    let first = f.remember("Giờ mở cửa", "Mở cửa lúc tám giờ").await;
    let receipt = f
        .core
        .apply(
            &f.scope,
            MemoryActor::Admin,
            MemoryChange::Discard {
                id: first.fact_id.clone().unwrap(),
                expected_revision: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(receipt.action, ReceiptAction::Discard);
    assert!(f.recall("tám").await.is_empty());
    assert!(f.fact_paths().is_empty());
    assert!(!f.context_rows().await.contains("Giờ mở cửa"));
    assert!(!f.summary_file().contains("Giờ mở cửa"));

    let again = f.remember("Giờ mở cửa", "Mở cửa lúc tám giờ").await;
    assert_ne!(again.fact_id, first.fact_id);
    assert_eq!(f.recall("tám").await.len(), 1);
    assert_eq!(
        f.tombstones_file(),
        "",
        "Discard leaves no tombstone at all"
    );
}

#[tokio::test]
async fn remember_after_forget_succeeds_and_lifts_the_tombstone() {
    let f = fixture();
    let first = f.remember("Mã cổng", "Mã cổng bảy bảy").await;
    f.forget(&first).await.unwrap();
    assert!(f.tombstones_file().contains("\"fp\""));

    let again = f.remember("Mã cổng", "Mã cổng bảy bảy").await;

    assert_ne!(again.fact_id, first.fact_id);
    assert!(!f.tombstones_file().contains("\"fp\""));
    assert_eq!(f.recall("cổng").await.len(), 1);
    assert!(f.context_rows().await.contains("Mã cổng"));
    f.core.reconcile(&f.scope).await.unwrap();
    assert_eq!(f.recall("cổng").await.len(), 1, "reconcile keeps it");
}

#[tokio::test]
async fn correct_onto_a_forgotten_body_lifts_the_tombstone_and_the_fact_stays_visible() {
    let f = fixture();
    let forgotten = f.remember("Cũ", "Mã cổng bảy bảy").await;
    f.forget(&forgotten).await.unwrap();
    let other = f.remember("Khác", "Nội dung tạm").await;

    let corrected = f
        .core
        .apply(
            &f.scope,
            MemoryActor::Admin,
            correct_change(&other, "Mã cổng bảy bảy"),
        )
        .await
        .unwrap();

    assert_eq!(corrected.revision, Some(2));
    assert!(!f.tombstones_file().contains("\"fp\""));
    assert_eq!(f.recall("cổng").await.len(), 1);
    f.core.reconcile(&f.scope).await.unwrap();
    assert_eq!(f.fact_paths().len(), 1, "reconcile does not delete it");
    assert_eq!(f.recall("cổng").await.len(), 1);
}

#[tokio::test]
async fn remember_after_a_crashed_forget_replaces_the_leftover_instead_of_reviving_it() {
    let f = fixture();
    let first = f.remember("Mã cổng", "Mã cổng bảy bảy").await;
    f.arm(FailPoint::Cleanup, FailAction::Crash);
    f.forget(&first)
        .await
        .expect_err("the crash stops the write");
    assert_eq!(f.fact_paths().len(), 1, "the leftover file is still there");

    let restarted = f.reopen();
    let again = remember_on(&restarted, &f.scope, "Mã cổng", "Mã cổng bảy bảy").await;

    assert_ne!(
        again.fact_id, first.fact_id,
        "not deduplicated against the leftover"
    );
    assert_eq!(f.fact_paths().len(), 1);
    assert_eq!(f.only_fact().await.id, again.fact_id.unwrap());
}

#[tokio::test]
async fn a_stale_expected_revision_is_a_conflict_and_leaves_the_file_unchanged() {
    let f = fixture();
    let first = f.remember("Giờ mở cửa", "Mở cửa lúc tám giờ").await;
    let second = f
        .core
        .apply(
            &f.scope,
            MemoryActor::Admin,
            correct_change(&first, "Mở cửa lúc chín giờ"),
        )
        .await
        .unwrap();
    assert_eq!(second.revision, Some(2));
    let before = fs::read(f.the_fact_path()).unwrap();
    let id = first.fact_id.clone().unwrap();

    for change in [
        correct_change(&first, "Bản ghi đè"),
        MemoryChange::Discard {
            id: id.clone(),
            expected_revision: 1,
        },
        MemoryChange::Forget {
            id: id.clone(),
            expected_revision: 1,
        },
    ] {
        let error = f
            .core
            .apply(&f.scope, MemoryActor::Admin, change)
            .await
            .expect_err("stale revision");
        assert_eq!(error.code(), "memory_revision_conflict");
        assert_eq!(fs::read(f.the_fact_path()).unwrap(), before);
    }
    assert_eq!(f.tombstones_file(), "");
}

#[tokio::test]
async fn correct_bumps_revision_applies_optional_fields_and_keeps_the_slug() {
    let f = fixture();
    let first = f.remember("Giờ mở cửa", "Mở cửa lúc tám giờ").await;
    let path = f.the_fact_path();
    let receipt = f
        .core
        .apply(
            &f.scope,
            MemoryActor::Admin,
            MemoryChange::Correct {
                id: first.fact_id.clone().unwrap(),
                expected_revision: 1,
                title: Some("Tiêu đề mới".into()),
                hook: Some("Móc mới".into()),
                body: "Mở cửa lúc chín giờ".into(),
            },
        )
        .await
        .unwrap();

    assert_eq!(receipt.action, ReceiptAction::Correct);
    assert_eq!(receipt.title.as_deref(), Some("Tiêu đề mới"));
    assert_eq!(
        f.the_fact_path(),
        path,
        "renaming a title never moves the file"
    );
    let fact = f.only_fact().await;
    assert_eq!((fact.revision, fact.hook.as_str()), (2, "Móc mới"));
    assert_eq!(f.recall("chín").await.len(), 1);
    assert!(
        f.recall("tám").await.is_empty(),
        "the index dropped the old body"
    );
    assert!(f.context_rows().await.contains("Tiêu đề mới"));
    assert!(f.summary_file().contains("Tiêu đề mới"));
}

#[tokio::test]
async fn set_pinned_keeps_the_revision_is_not_an_owner_edit_and_leads_the_summary() {
    let f = fixture();
    let older = f.remember("Cũ hơn", "Thân cũ hơn").await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    f.remember("Mới hơn", "Thân mới hơn").await;
    let first_row = |rows: String| rows.lines().next().unwrap_or_default().to_string();
    assert!(first_row(f.context_rows().await).contains("Mới hơn"));

    let receipt = f
        .core
        .apply(
            &f.scope,
            MemoryActor::Admin,
            MemoryChange::SetPinned {
                id: older.fact_id.clone().unwrap(),
                pinned: true,
            },
        )
        .await
        .unwrap();

    assert_eq!(receipt.revision, Some(1));
    assert!(first_row(f.context_rows().await).contains("Cũ hơn"));
    let report = f.core.reconcile(&f.scope).await.unwrap();
    assert_eq!(report.owner_edited, 0);
    // Pinning does not conflict with someone editing from the same revision.
    f.core
        .apply(
            &f.scope,
            MemoryActor::Admin,
            correct_change(&older, "Thân đã sửa"),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn two_titles_with_one_slug_make_two_files() {
    let f = fixture();
    let first = f.remember("Đặt lịch", "Nội dung một").await;
    let second = f.remember("Dat Lich", "Nội dung hai").await;
    assert_ne!(first.fact_id, second.fact_id);
    let names: Vec<String> = f
        .fact_paths()
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    let suffix = &second.fact_id.unwrap().as_str()["fact_".len()..]
        .chars()
        .take(6)
        .collect::<String>();
    assert_eq!(
        names,
        [format!("dat-lich-{suffix}.md"), "dat-lich.md".to_string()]
    );
    assert_eq!(f.recall("một").await.len(), 1);
    assert_eq!(f.recall("hai").await.len(), 1);
}

#[tokio::test]
async fn remembering_the_same_body_returns_the_existing_fact() {
    let f = fixture();
    let first = f.remember("Giờ mở cửa", "Mở cửa lúc tám giờ").await;
    let resent = f.remember("Tiêu đề khác", "  mo cua luc TAM gio\r\n").await;
    assert_eq!(resent.fact_id, first.fact_id);
    assert_eq!(resent.revision, Some(1));
    assert_eq!(resent.title.as_deref(), Some("Giờ mở cửa"));
    assert_eq!(f.fact_paths().len(), 1);
}

#[tokio::test]
async fn sizes_secrets_and_the_fact_limit_are_refused_and_nothing_is_written() {
    let f = fixture_with(|cfg| {
        cfg.max_facts_per_scope = 1;
        cfg.max_fact_body_chars = 20;
    });
    for (change, code) in [
        (
            remember_change(&"t".repeat(121), "thân"),
            "memory_too_large",
        ),
        (remember_change("t", &"b".repeat(21)), "memory_too_large"),
        (
            remember_change("t", "AKIAIOSFODNN7EXAMPLE"),
            "memory_rejected_sensitive",
        ),
        (
            remember_change("sk-abcdefghijklmnopqrstuvwx", "thân"),
            "memory_rejected_sensitive",
        ),
    ] {
        let error = f
            .core
            .apply(&f.scope, MemoryActor::Admin, change)
            .await
            .expect_err(code);
        assert_eq!(error.code(), code);
    }
    assert!(f.fact_paths().is_empty());

    let kept = f.remember("Một", "Thân một").await;
    let error = f
        .core
        .apply(
            &f.scope,
            MemoryActor::Admin,
            remember_change("Hai", "Thân hai"),
        )
        .await
        .expect_err("limit");
    assert_eq!(error.code(), "memory_limit_reached");
    // Resending what is already saved still answers, because it adds nothing.
    assert_eq!(f.remember("Một", "Thân một").await.fact_id, kept.fact_id);
    let error = f
        .core
        .apply(
            &f.scope,
            MemoryActor::Admin,
            correct_change(&kept, "AKIAIOSFODNN7EXAMPLE"),
        )
        .await
        .expect_err("guard covers Correct");
    assert_eq!(error.code(), "memory_rejected_sensitive");
}

#[tokio::test]
async fn ordinary_text_with_the_word_password_is_saved() {
    let f = fixture();
    f.remember("Đổi password", "Nhớ đổi password wifi mỗi tháng")
        .await;
    assert_eq!(f.recall("password").await.len(), 1);
}

#[tokio::test]
async fn unknown_ids_are_not_found() {
    let f = fixture();
    let missing = FactId::parse(&format!("fact_{}", "a".repeat(32))).unwrap();
    for change in [
        MemoryChange::Correct {
            id: missing.clone(),
            expected_revision: 1,
            title: None,
            hook: None,
            body: "x".into(),
        },
        MemoryChange::Discard {
            id: missing.clone(),
            expected_revision: 1,
        },
        MemoryChange::Forget {
            id: missing.clone(),
            expected_revision: 1,
        },
        MemoryChange::SetPinned {
            id: missing.clone(),
            pinned: true,
        },
    ] {
        let error = f
            .core
            .apply(&f.scope, MemoryActor::Admin, change)
            .await
            .expect_err("no such fact");
        assert_eq!(error.code(), "memory_not_found");
    }
    let error = f
        .core
        .inspect(&f.scope, InspectFilter { id: Some(missing) })
        .await
        .expect_err("no such fact");
    assert_eq!(error.code(), "memory_not_found");
}

#[test]
fn fact_ids_other_than_fact_uuid_do_not_parse() {
    assert!(FactId::parse(&format!("fact_{}", "0a".repeat(16))).is_some());
    for id in [
        "wiki/entities/page",
        "entities/fact_x",
        "fact_",
        "fact_../../etc/passwd",
        "fact_ABCDEF00000000000000000000000000",
        "fact_00000000000000000000000000000000x",
    ] {
        assert!(FactId::parse(id).is_none(), "{id}");
    }
}

#[tokio::test]
async fn purge_scope_empties_open_contexts_removes_the_directory_and_stays_recallable() {
    let f = fixture();
    f.remember("Mã cổng", "Mã cổng bảy bảy").await;
    let handle = f.core.context(&f.scope).await;
    let receipt = f
        .core
        .apply(&f.scope, MemoryActor::Admin, MemoryChange::PurgeScope)
        .await
        .unwrap();
    assert_eq!(receipt.action, ReceiptAction::PurgeScope);
    assert!(!receipt.cleanup_pending);
    assert!(handle.current().rows.is_empty());
    assert!(!f.device_dir().exists());
    assert!(f.recall("cổng").await.is_empty());
    assert!(
        !f.device_dir().exists(),
        "reading does not recreate a vault"
    );
}

#[tokio::test]
async fn purge_failure_after_publish_reports_cleanup_pending() {
    let f = fixture();
    f.remember("Mã cổng", "Mã cổng bảy bảy").await;
    let handle = f.core.context(&f.scope).await;
    f.arm(FailPoint::Cleanup, FailAction::Error);
    let receipt = f
        .core
        .apply(&f.scope, MemoryActor::Admin, MemoryChange::PurgeScope)
        .await
        .unwrap();
    assert!(receipt.cleanup_pending);
    assert!(handle.current().rows.is_empty());
    assert!(f.device_dir().exists());
}

#[tokio::test]
async fn set_profile_checks_revision_guards_content_and_republishes_every_open_scope() {
    let f = fixture();
    let deployment = MemoryScope::deployment();
    f.remember("Giờ mở cửa", "Mở cửa lúc tám giờ").await;
    let handle = f.core.context(&f.scope).await;

    let receipt = f
        .core
        .apply(
            &deployment,
            MemoryActor::Admin,
            MemoryChange::SetProfile {
                body: "Cửa hàng tại Hà Nội".into(),
                expected_revision: 0,
            },
        )
        .await
        .unwrap();

    assert_eq!(receipt.action, ReceiptAction::SetProfile);
    assert_eq!(receipt.revision, Some(1));
    assert!(
        handle
            .current()
            .rows
            .join("\n")
            .contains("Cửa hàng tại Hà Nội")
    );
    assert!(handle.current().rows.join("\n").contains("Giờ mở cửa"));
    let profile = f
        .core
        .inspect(&deployment, InspectFilter::default())
        .await
        .unwrap()
        .profile
        .unwrap();
    assert_eq!(
        (profile.body.as_str(), profile.revision),
        ("Cửa hàng tại Hà Nội", 1)
    );
    assert!(profile.updated_at.is_some());

    for (body, revision, code) in [
        ("x", 0, "memory_revision_conflict"),
        ("AKIAIOSFODNN7EXAMPLE", 1, "memory_rejected_sensitive"),
    ] {
        let error = f
            .core
            .apply(
                &deployment,
                MemoryActor::Admin,
                MemoryChange::SetProfile {
                    body: body.into(),
                    expected_revision: revision,
                },
            )
            .await
            .expect_err(code);
        assert_eq!(error.code(), code);
    }
}
