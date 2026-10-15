use super::*;

#[tokio::test]
async fn artifact_store_rejects_another_database_before_reconciliation() {
    let (db, _, runtime) = harness().await;
    let root = super::super::artifacts::root(&db).unwrap();
    let artifact = root
        .join(uuid::Uuid::new_v4().to_string())
        .join("committed");
    tokio::fs::create_dir_all(artifact.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&artifact, b"owned bytes").await.unwrap();
    runtime.shutdown().await;
    let mut other_config = config();
    other_config.database.url = format!(
        "sqlite://{}",
        db.durable_path()
            .unwrap()
            .with_file_name("other.db")
            .display()
    );
    let other_db = Arc::new(Database::connect(&other_config.database).await.unwrap());
    assert!(matches!(
        BackgroundRuntime::foundation_harness(other_db, Arc::new(other_config)).await,
        Err(JobError::DeploymentLock)
    ));
    assert_eq!(tokio::fs::read(&artifact).await.unwrap(), b"owned bytes");
}

#[tokio::test]
async fn shutdown_drains_blocked_submission_before_releasing_deployment_ownership() {
    let (db, config, runtime) = harness().await;
    let publication = db.tool_security.publication.clone().write_owned().await;
    let engine = runtime.engine.clone();
    let submit = tokio::spawn(async move {
        engine
            .submit(
                request("Blocked at revocation boundary"),
                &JobRequester::Admin,
                None,
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while runtime.engine.active_submissions() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let stopping = runtime.clone();
    let mut shutdown = tokio::spawn(async move { stopping.shutdown().await });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut shutdown)
            .await
            .is_err(),
        "shutdown released ownership before the submit ended"
    );
    assert!(matches!(
        BackgroundRuntime::foundation_harness(db.clone(), config.clone()).await,
        Err(JobError::DeploymentLock)
    ));
    drop(publication);
    assert_eq!(
        submit.await.unwrap().unwrap_err(),
        JobError::RuntimeUnavailable
    );
    shutdown.await.unwrap();
    let next = BackgroundRuntime::foundation_harness(db, config)
        .await
        .unwrap();
    assert_eq!(next.engine.list(1, 30, None, None).await.unwrap().1, 0);
    next.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deployment_lock_excludes_a_second_process_and_releases_after_shutdown() {
    let (_, config, runtime) = harness().await;
    for expected in ["locked", "released"] {
        if expected == "released" {
            runtime.shutdown().await;
        }
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("services::background::tests::locking::background_lock_child_entry")
            .env("BACKGROUND_LOCK_PROBE_URL", &config.database.url)
            .env("BACKGROUND_LOCK_PROBE_EXPECTED", expected)
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "child lock qualification failed: {}",
            String::from_utf8_lossy(&child.stderr)
        );
    }
}
#[tokio::test]
async fn background_lock_child_entry() {
    let Ok(url) = std::env::var("BACKGROUND_LOCK_PROBE_URL") else {
        return;
    };
    let mut config = config();
    config.database.url = url;
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let result = BackgroundRuntime::open(
        db,
        Arc::new(config),
        tokio_util::sync::CancellationToken::new(),
    )
    .await;
    if std::env::var("BACKGROUND_LOCK_PROBE_EXPECTED").unwrap() == "locked" {
        assert!(matches!(result, Err(JobError::DeploymentLock)));
    } else {
        result.unwrap().shutdown().await;
    }
}

#[tokio::test]
async fn live_device_and_agent_mutations_wait_for_the_shared_publication_gate() {
    let (db, _, runtime) = harness().await;
    let requester = voice(&db).await;
    let JobRequester::Voice {
        device_incarnation,
        agent_incarnation,
    } = requester
    else {
        unreachable!()
    };
    let read = db.tool_security.publication.clone().read_owned().await;
    let device_db = db.clone();
    let mut device = tokio::spawn(async move {
        device_db
            .update_device(
                device_incarnation,
                1,
                crate::database::devices::DeviceChanges {
                    agent_key: "voice-agent",
                    template_key: None,
                    name: None,
                    description: None,
                    metadata: None,
                    enabled: 0,
                },
                "background-revoke",
            )
            .await
    });
    // The commit must remain pending while a worker-start publication reader owns authority.
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut device)
            .await
            .is_err()
    );
    drop(read);
    device.await.unwrap().unwrap();
    assert_eq!(
        runtime
            .engine
            .submit(
                request("Disabled Device"),
                &JobRequester::Voice {
                    device_incarnation,
                    agent_incarnation
                },
                None
            )
            .await
            .unwrap_err(),
        JobError::AuthorityRevoked
    );
    let read = db.tool_security.publication.clone().read_owned().await;
    let agent_db = db.clone();
    let mut agent = tokio::spawn(async move {
        agent_db
            .update_agent(
                agent_incarnation,
                1,
                crate::database::agents::AgentChanges {
                    name: "Voice",
                    description: None,
                    enabled: 0,
                },
                "background-agent-revoke",
            )
            .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut agent)
            .await
            .is_err()
    );
    drop(read);
    agent.await.unwrap().unwrap();
    runtime.shutdown().await;
}

#[test]
fn background_provider_admission_cannot_use_voice_or_diagnostic_single_slot_exception() {
    use crate::workers::{ProviderRuntimeAdmission, ProviderWorkloadClass};
    let single = ProviderRuntimeAdmission::new(1, 1);
    assert!(single.try_admit(ProviderWorkloadClass::Background).is_err());
    assert!(single.try_admit(ProviderWorkloadClass::Diagnostic).is_ok());
    let shared = ProviderRuntimeAdmission::new(3, 1);
    let first = shared.try_admit(ProviderWorkloadClass::Background).unwrap();
    let second = shared.try_admit(ProviderWorkloadClass::Background).unwrap();
    assert!(shared.try_admit(ProviderWorkloadClass::Background).is_err());
    assert!(shared.try_admit(ProviderWorkloadClass::Voice).is_ok());
    drop((first, second));
}
