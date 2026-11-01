//! Who may do what, on which scope, and the error contract callers map from.

use super::*;

const ALL_INTENTS: ExplicitIntent = ExplicitIntent {
    remember: true,
    correct: true,
    forget: true,
};

fn admin_only_changes(id: &FactId) -> [MemoryChange; 3] {
    [
        MemoryChange::Discard {
            id: id.clone(),
            expected_revision: 1,
        },
        MemoryChange::SetPinned {
            id: id.clone(),
            pinned: true,
        },
        MemoryChange::PurgeScope,
    ]
}

#[tokio::test]
async fn a_voice_user_can_never_discard_pin_set_profile_or_purge() {
    let f = fixture();
    let fact = f.remember("Mã cổng", "Mã cổng bảy bảy").await;
    let actor = voice(MemoryMode::ReadWrite, ALL_INTENTS);
    for change in admin_only_changes(&fact.fact_id.clone().unwrap()) {
        let error = f
            .core
            .apply(&f.scope, actor, change)
            .await
            .expect_err("Admin only");
        assert_eq!(error.code(), "memory_scope_denied");
    }
    let error = f
        .core
        .apply(
            &MemoryScope::deployment(),
            actor,
            MemoryChange::SetProfile {
                body: "x".into(),
                expected_revision: 0,
            },
        )
        .await
        .expect_err("Admin only");
    assert_eq!(error.code(), "memory_scope_denied");
    assert_eq!(f.fact_paths().len(), 1);
}

#[tokio::test]
async fn each_voice_write_needs_its_own_intent_flag() {
    let f = fixture();
    let fact = f.remember("Mã cổng", "Mã cổng bảy bảy").await;
    let id = fact.fact_id.clone().unwrap();
    let remember_only = ExplicitIntent {
        remember: true,
        ..ExplicitIntent::default()
    };
    let correct_only = ExplicitIntent {
        correct: true,
        ..ExplicitIntent::default()
    };
    let forget_only = ExplicitIntent {
        forget: true,
        ..ExplicitIntent::default()
    };
    let forget = || MemoryChange::Forget {
        id: id.clone(),
        expected_revision: 1,
    };
    let refused = [
        (remember_only, forget()),
        (remember_only, correct_change(&fact, "Sửa")),
        (correct_only, forget()),
        (correct_only, remember_change("T", "Thân mới")),
        (forget_only, correct_change(&fact, "Sửa")),
        (forget_only, remember_change("T", "Thân mới")),
        (ExplicitIntent::default(), remember_change("T", "Thân mới")),
    ];
    for (intent, change) in refused {
        let error = f
            .core
            .apply(&f.scope, voice(MemoryMode::ReadWrite, intent), change)
            .await
            .expect_err("wrong flag");
        assert_eq!(error.code(), "memory_requires_explicit_request");
    }
    assert_eq!(f.only_fact().await.revision, 1);

    let remembered = f
        .core
        .apply(
            &f.scope,
            voice(MemoryMode::ReadWrite, remember_only),
            remember_change("Mới", "Thân mới"),
        )
        .await
        .unwrap();
    let corrected = f
        .core
        .apply(
            &f.scope,
            voice(MemoryMode::ReadWrite, correct_only),
            correct_change(&remembered, "Thân đã sửa"),
        )
        .await
        .unwrap();
    assert_eq!(corrected.revision, Some(2));
    f.core
        .apply(
            &f.scope,
            voice(MemoryMode::ReadWrite, forget_only),
            forget(),
        )
        .await
        .unwrap();
    assert!(f.recall("cổng").await.is_empty());
}

#[tokio::test]
async fn a_voice_write_records_where_it_came_from() {
    let f = fixture();
    let intent = ExplicitIntent {
        remember: true,
        ..ExplicitIntent::default()
    };
    f.core
        .apply(
            &f.scope,
            voice(MemoryMode::ReadWrite, intent),
            remember_change("T", "Thân"),
        )
        .await
        .unwrap();
    let fact = f.only_fact().await;
    assert_eq!(fact.origin, FactOrigin::Voice);
    assert_eq!(fact.source_ref.as_deref(), Some("device:7"));
}

#[tokio::test]
async fn device_memory_mode_gates_voice_writes_but_never_admin() {
    let f = fixture();
    for (mode, code) in [
        (MemoryMode::Off, "memory_disabled"),
        (MemoryMode::Read, "memory_read_only"),
    ] {
        let error = f
            .core
            .apply(
                &f.scope,
                voice(mode, ALL_INTENTS),
                remember_change("T", "Thân"),
            )
            .await
            .expect_err(code);
        assert_eq!(error.code(), code);
    }
    assert!(f.fact_paths().is_empty());
    // Admin carries no mode: it may prepare data while the Device is still off.
    f.remember("T", "Thân").await;
}

#[tokio::test]
async fn the_deployment_scope_only_takes_set_profile_and_inspect() {
    let f = fixture();
    let deployment = MemoryScope::deployment();
    let id = FactId::parse(&format!("fact_{}", "a".repeat(32))).unwrap();
    let [discard, pin, purge] = admin_only_changes(&id);
    for change in [
        remember_change("T", "Thân"),
        MemoryChange::Correct {
            id: id.clone(),
            expected_revision: 1,
            title: None,
            hook: None,
            body: "x".into(),
        },
        MemoryChange::Forget {
            id: id.clone(),
            expected_revision: 1,
        },
        discard,
        pin,
        purge,
    ] {
        let error = f
            .core
            .apply(&deployment, MemoryActor::Admin, change)
            .await
            .expect_err("deployment takes only SetProfile");
        assert_eq!(error.code(), "memory_scope_denied");
    }
    let error = f
        .core
        .recall(
            &deployment,
            RecallQuery {
                text: "x".into(),
                limit: 5,
            },
        )
        .await
        .expect_err("no recall on deployment");
    assert_eq!(error.code(), "memory_scope_denied");
    let error = f
        .core
        .reconcile(&deployment)
        .await
        .expect_err("no reconcile");
    assert_eq!(error.code(), "memory_scope_denied");
    assert!(
        f.core
            .inspect(&deployment, InspectFilter::default())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn a_device_scope_refuses_set_profile() {
    let f = fixture();
    let error = f
        .core
        .apply(
            &f.scope,
            MemoryActor::Admin,
            MemoryChange::SetProfile {
                body: "x".into(),
                expected_revision: 0,
            },
        )
        .await
        .expect_err("profile lives on the deployment scope");
    assert_eq!(error.code(), "memory_scope_denied");
}

#[test]
fn stored_device_kinds_map_onto_the_interface_type() {
    assert_eq!(
        DeviceKind::from_stored("web_assistant"),
        DeviceKind::Virtual
    );
    assert_eq!(DeviceKind::from_stored("physical"), DeviceKind::Physical);
}

#[test]
fn opening_with_memory_disabled_is_refused_and_touches_no_disk() {
    let root = std::env::temp_dir().join(format!("memory-core-{}", uuid::Uuid::new_v4()));
    let cfg = MemorySectionConfig {
        vault_dir: root.clone(),
        ..MemorySectionConfig::default()
    };
    assert!(!cfg.enabled);
    assert_eq!(
        MemoryCore::open(&cfg).err().map(MemoryError::code),
        Some("memory_disabled")
    );
    assert!(!root.exists());
}

#[test]
fn error_codes_and_statuses_match_the_contract_table() {
    for (error, code, status) in [
        (MemoryError::Disabled, "memory_disabled", 503),
        (MemoryError::ReadOnly, "memory_read_only", 403),
        (
            MemoryError::RequiresExplicitRequest,
            "memory_requires_explicit_request",
            403,
        ),
        (
            MemoryError::RevisionConflict,
            "memory_revision_conflict",
            409,
        ),
        (MemoryError::NotFound, "memory_not_found", 404),
        (MemoryError::LimitReached, "memory_limit_reached", 409),
        (MemoryError::TooLarge, "memory_too_large", 400),
        (
            MemoryError::RejectedSensitive,
            "memory_rejected_sensitive",
            400,
        ),
        (MemoryError::ScopeDenied, "memory_scope_denied", 403),
        (MemoryError::Unavailable, "memory_unavailable", 503),
    ] {
        assert_eq!(error.code(), code);
        assert_eq!(error.http_status().as_u16(), status, "{code}");
    }
}
