use super::*;

#[tokio::test]
async fn a_cold_switch_prepares_outside_actor_and_commits_only_at_normal_writer_boundary() {
    let (mut actor, manager) = managed_switch_actor();
    let turn_id = actor.begin_active_turn().unwrap();
    actor.execute_template_switch(ToolCall {
        id: "cold-switch".into(),
        name: SWITCH_TEMPLATE_TOOL_NAME.into(),
        arguments: serde_json::json!({"template":"cold"}),
    });
    assert_eq!(actor.profile_revision(), 1);
    assert!(actor.template_prepare.is_some());
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    while actor.pending_actions.switch_template_after_turn.is_none() {
        actor.drain_template_preparation();
        assert!(tokio::time::Instant::now() < deadline);
        tokio::task::yield_now().await;
    }
    assert_eq!(actor.profile_revision(), 1);
    actor.on_writer_event(WriterEvent::TurnClosed {
        turn_id,
        outcome: WriterTurnOutcome::Normal,
    });
    actor.drain_managed_switch_boundary();
    assert_eq!(actor.profile_revision(), 2);
    assert_eq!(actor.profile.system_prompt, "cold prompt");
    assert_eq!(manager.accounting().active_leases, 4);
    drop(actor);
    assert_eq!(manager.accounting().active_leases, 0);
}

#[tokio::test]
async fn abort_discards_cold_switch_preparation_and_every_late_lease() {
    let (mut actor, manager) = managed_switch_actor();
    actor.begin_active_turn().unwrap();
    actor.execute_template_switch(ToolCall {
        id: "cold-switch".into(),
        name: SWITCH_TEMPLATE_TOOL_NAME.into(),
        arguments: serde_json::json!({"template":"cold"}),
    });
    actor.interrupt_active_turn();
    actor.drain_template_preparation();
    assert_eq!(actor.profile_revision(), 1);
    assert!(actor.template_prepare.is_none());
    assert!(actor.pending_actions.switch_template_after_turn.is_none());
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    assert_eq!(manager.accounting().active_leases, 0);
}

#[tokio::test]
async fn managed_switch_keeps_old_profile_until_exact_vad_cleanup_acknowledgement() {
    let (mut actor, _) = managed_switch_actor();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    actor.vad_runtime = Arc::new(VadWorkerRuntime::new(
        Arc::new(CloseGateVad {
            entered: entered_tx,
            release: Arc::new(std::sync::Mutex::new(release_rx)),
        }),
        worker(),
    ));
    actor.vad_events = actor.vad_runtime.register_session(&actor.session_id);
    let identity = WorkerIdentity::new(actor.session_id.clone(), actor.generation, 1);
    let lease = actor.vad_runtime.open(identity.clone()).unwrap();
    actor.vad_session = Some((lease, identity));
    let turn_id = actor.begin_active_turn().unwrap();
    actor.execute_template_switch(ToolCall {
        id: "close-gate-switch".into(),
        name: SWITCH_TEMPLATE_TOOL_NAME.into(),
        arguments: serde_json::json!({"template":"cold"}),
    });
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    while actor.pending_actions.switch_template_after_turn.is_none() {
        actor.drain_template_preparation();
        assert!(tokio::time::Instant::now() < deadline);
        tokio::task::yield_now().await;
    }
    actor.on_writer_event(WriterEvent::TurnClosed {
        turn_id,
        outcome: WriterTurnOutcome::Normal,
    });
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    actor.pump_workers();
    assert_eq!(actor.profile_revision(), 1);
    assert!(actor.managed_switch_boundary.is_some());
    actor.on_client_message(ClientMessage::Abort { session_id: None });
    assert!(
        actor.managed_switch_boundary.is_some(),
        "a sealed Normal outcome cannot be undone by a later abort while native cleanup finishes"
    );
    actor.on_client_message(ClientMessage::Listen {
        session_id: None,
        command: ListenCommand::Start {
            mode: ListenMode::Manual,
        },
    });
    release_tx.send(()).unwrap();
    while actor.profile_revision() == 1 {
        actor.pump_workers();
        assert!(tokio::time::Instant::now() < deadline);
        tokio::task::yield_now().await;
    }
    assert_eq!(actor.profile.system_prompt, "cold prompt");
    assert!(actor.deferred_switch_ingress.is_empty());
    assert_eq!(actor.listening_mode, Some(ListenMode::Manual));
    assert_eq!(actor.phase, SessionPhase::Listening);
}

#[tokio::test]
async fn abort_while_native_switch_build_is_blocked_cannot_commit_late_completion() {
    use crate::services::provider_runtime::{ProviderRuntimeManager, RuntimeLimits, RuntimeState};
    let (mut actor, _) = managed_switch_actor();
    let (entered, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 0,
            max_waiters: 8,
            max_resident_bytes: 8,
            max_resources: 8,
            max_version_entries: 16,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 10,
            idle_ttl_ms: 1000,
        },
        Arc::new(HeldSwitchBuilder {
            catalog: catalog(),
            entered,
            release: std::sync::Mutex::new(release_rx),
        }),
        AdmissionGate::open(),
    )
    .unwrap();
    actor.switch_catalog.manager = Some(manager.clone());
    actor.begin_active_turn().unwrap();
    actor.execute_template_switch(ToolCall {
        id: "held-switch".into(),
        name: SWITCH_TEMPLATE_TOOL_NAME.into(),
        arguments: serde_json::json!({"template":"cold"}),
    });
    entered_rx.recv().await.unwrap();
    actor.interrupt_active_turn();
    assert!(actor.template_prepare.is_none());
    assert_eq!(actor.profile_revision(), 1);
    release.send(()).unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    while manager.inspect(2, 2).desired_state != RuntimeState::Ready {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::task::yield_now().await;
    }
    actor.drain_template_preparation();
    assert_eq!(actor.profile_revision(), 1);
    assert!(actor.pending_actions.switch_template_after_turn.is_none());
    assert_eq!(manager.accounting().active_leases, 0);
    assert_eq!(manager.accounting().build_attempts, 1);
}

#[tokio::test]
async fn mixed_source_candidate_acquires_exact_versions_and_releases_every_lease() {
    use crate::services::provider_runtime::{ProviderIdentity, ProviderVersion};
    let mut catalog = catalog();
    let speaker = Arc::new(
        crate::providers::speaker::SpeakerRuntime::new(
            Box::new(SwitchSpeaker),
            crate::workers::ProviderRuntimeAdmission::new(1, 1),
        )
        .unwrap(),
    );
    catalog.speaker.insert("voice".into(), speaker.clone());
    let manager = switch_manager(catalog.clone());
    let mut actor = admitted_actor(&catalog);
    actor.switch_catalog.manager = Some(manager.clone());
    let mut assignment = candidate(8, "mixed", "mixed prompt", "alternate", "alternate-vad");
    for (index, binding) in assignment.bindings.iter_mut().enumerate() {
        binding.snapshot = Some(Arc::new(crate::database::DesiredProvider {
            id: index as i64 + 1,
            key: binding.provider_key.clone(),
            kind: binding.provider_type.clone(),
            adapter: "fixture".into(),
            revision: 2,
            config_json: "{}".into(),
            secret_ref: None,
        }));
    }
    let db = crate::session::ConfiguredTemplateProfile::from_assignment(&assignment).unwrap();
    let llm = assignment
        .bindings
        .iter_mut()
        .find(|b| b.provider_type == "llm")
        .unwrap();
    let snapshot = crate::database::DesiredProvider {
        id: 0,
        key: llm.provider_key.clone(),
        kind: "llm".into(),
        adapter: "fixture".into(),
        revision: 1,
        config_json: "{}".into(),
        secret_ref: None,
    };
    llm.provider_key = format!("config:llm:{}", snapshot.key);
    llm.snapshot = None;
    assignment.bindings.push(AdmittedProviderBinding {
        provider_type: "speaker".into(),
        provider_key: "config:speaker:voice".into(),
        provider_enabled: true,
        snapshot: None,
    });
    let deployment_speaker = crate::database::DesiredProvider {
        id: 0,
        key: "voice".into(),
        kind: "speaker".into(),
        adapter: "fixture".into(),
        revision: 1,
        config_json: "{}".into(),
        secret_ref: None,
    };
    let mixed = crate::session::ConfiguredTemplateProfile::from_assignment_with_defaults(
        &assignment,
        &deployment().provider_defaults.effective_bindings(),
        &[snapshot, deployment_speaker],
    );
    let mixed = mixed.unwrap();
    let old = db
        .prepare(&manager, manager.admission_deadline())
        .await
        .unwrap();
    let prepared = mixed
        .prepare(&manager, manager.admission_deadline())
        .await
        .unwrap();
    assert_eq!(prepared.leases.len(), 5);
    assert!(Arc::ptr_eq(
        prepared.runtimes.speaker.as_ref().unwrap(),
        &speaker
    ));
    assert!(prepared.leases.iter().any(|l| l.version()
        == &ProviderVersion {
            identity: ProviderIdentity::Deployment {
                kind: "llm".into(),
                key: "alternate".into()
            },
            revision: 1
        }));
    assert!(
        prepared
            .leases
            .iter()
            .filter(|l| matches!(l.version().identity, ProviderIdentity::Database(_)))
            .all(|l| l.version().revision == 2)
    );
    assert_eq!(manager.accounting().active_leases, 9);
    drop(old);
    drop(prepared);
    actor.switch_catalog.cold.push(mixed);
    let turn_id = actor.begin_active_turn().unwrap();
    actor.execute_template_switch(ToolCall {
        id: "mixed-switch".into(),
        name: SWITCH_TEMPLATE_TOOL_NAME.into(),
        arguments: serde_json::json!({"template":"mixed"}),
    });
    let deadline = manager.admission_deadline();
    while actor.pending_actions.switch_template_after_turn.is_none() {
        actor.drain_template_preparation();
        assert!(tokio::time::Instant::now() < deadline);
        tokio::task::yield_now().await;
    }
    assert_eq!(actor.profile_revision(), 1);
    actor.on_writer_event(WriterEvent::TurnClosed {
        turn_id,
        outcome: WriterTurnOutcome::Normal,
    });
    actor.drain_managed_switch_boundary();
    assert_eq!(actor.profile_revision(), 2);
    assert_eq!(actor.profile.system_prompt, "mixed prompt");
    assert_eq!(manager.accounting().active_leases, 5);
    drop(actor);
    assert_eq!(manager.accounting().active_leases, 0);
}
