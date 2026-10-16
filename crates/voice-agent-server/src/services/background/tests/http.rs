use super::*;
use crate::{
    app::{AppState, router_with_state},
    providers::ProviderSet,
};

#[tokio::test]
async fn admin_artifact_download_is_authenticated_and_scoped_to_job() {
    use sha2::{Digest, Sha256};
    let (db, config, runtime) = harness().await;
    let job = runtime
        .engine
        .submit(
            request("Download checked result"),
            &JobRequester::Admin,
            None,
        )
        .await
        .unwrap()
        .job;
    let artifact_id = uuid::Uuid::new_v4().to_string();
    let bytes = b"fixture result";
    let directory = super::super::artifacts::root(&db).unwrap().join(&job.id);
    tokio::fs::create_dir_all(&directory).await.unwrap();
    tokio::fs::write(directory.join(&artifact_id), bytes)
        .await
        .unwrap();
    let sha: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    sqlx::query("INSERT INTO job_artifacts(id,job_id,relative_path,media_type,size_bytes,sha256,created_at) VALUES(?,?,?,?,?,?,0)")
        .bind(&artifact_id).bind(&job.id).bind(format!("{}/{artifact_id}", job.id))
        .bind("text/plain").bind(bytes.len() as i64).bind(sha)
        .execute(db.pool()).await.unwrap();
    sqlx::query("UPDATE jobs SET status='succeeded',result_json=?,plan_revision=1 WHERE id=?")
        .bind(json!({"summary":"fixture result"}).to_string())
        .bind(&job.id)
        .execute(db.pool())
        .await
        .unwrap();
    let mut state = AppState::from_provider_set_with_database(
        (*config).clone(),
        Arc::new(ProviderSet::unavailable()),
        Some((*db).clone()),
    );
    state.background = Some(runtime.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin/jobs", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    let client = reqwest::Client::new();
    let detail: Value = client
        .get(format!("{base}/{}", job.id))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(detail["result"]["summary"], "fixture result");
    assert_eq!(detail["plan_revision"], 1);
    let url = format!("{base}/{}/artifacts/{artifact_id}", job.id);
    assert_eq!(client.get(&url).send().await.unwrap().status(), 401);
    let response = client
        .get(&url)
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.bytes().await.unwrap().as_ref(), bytes);
    let other = uuid::Uuid::new_v4();
    assert_eq!(
        client
            .get(format!("{base}/{other}/artifacts/{artifact_id}"))
            .bearer_auth("background-test-token")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    server.abort();
    runtime.shutdown().await;
}

#[tokio::test]
async fn plugin_upload_installs_and_updates_native_package_without_losing_valid_installation() {
    use sha2::{Digest, Sha256};
    use std::io::{Cursor, Write};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn package(version: &str) -> Vec<u8> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../plugins/fixture-rust-1.0.0");
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for name in [
            "plugin.toml",
            "instructions.md",
            "schemas/collect.input.json",
            "schemas/collect.output.json",
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            let text = std::fs::read_to_string(root.join(name)).unwrap();
            zip.write_all(text.replace("1.0.0", version).as_bytes())
                .unwrap();
        }
        // Only package admission is under test; the Installer RPC is stubbed.
        let mut elf = [0u8; 64];
        elf[..6].copy_from_slice(b"\x7fELF\x02\x01");
        elf[16] = 2;
        let machine: u16 = match std::env::consts::ARCH {
            "x86_64" => 62,
            "aarch64" => 183,
            arch => panic!("unsupported plugin architecture: {arch}"),
        };
        elf[18..20].copy_from_slice(&machine.to_le_bytes());
        zip.start_file("bin/plugin", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&elf).unwrap();
        zip.finish().unwrap().into_inner()
    }

    let directory = std::env::temp_dir().join(format!("plugin-http-{}", uuid::Uuid::new_v4()));
    let mut config = super::config();
    config.database.url = format!("sqlite://{}", directory.join("test.db").display());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let root = directory.join("plugins");
    let socket = std::env::temp_dir().join(format!("plugin-{}.sock", uuid::Uuid::new_v4()));
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    config.background.plugin_installer_socket = Some(socket.clone());
    config.background.plugin_installations_dir = Some(root.join("installations"));
    let installations = config.background.plugin_installations_dir.clone().unwrap();
    let installer = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).await.unwrap();
            let request: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(request["action"], "Prepare");
            let digest = request["package_digest"].as_str().unwrap();
            let installation: String = Sha256::digest(format!("{digest}:shared-v1"))
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            let receipt = json!({"installation_digest":installation,"package_digest":digest});
            let directory = installations.join(&installation);
            tokio::fs::create_dir_all(&directory).await.unwrap();
            tokio::fs::write(directory.join(".installation.json"), receipt.to_string())
                .await
                .unwrap();
            let reply = json!({"ok":true,"value":receipt});
            stream
                .write_all(format!("{reply}\n").as_bytes())
                .await
                .unwrap();
        }
    });
    let runtime = BackgroundRuntime::foundation_harness(db.clone(), Arc::new(config.clone()))
        .await
        .unwrap();
    let mut state = AppState::from_provider_set_with_database(
        config,
        Arc::new(ProviderSet::unavailable()),
        Some((*db).clone()),
    );
    state.background = Some(runtime.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://{}/api/admin/plugins",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap();
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let mut previous_digest = Value::Null;
    for (version, revision) in [("1.0.0", 1), ("1.0.1", 3)] {
        let response = client
            .post(format!("{base}/install"))
            .bearer_auth("background-test-token")
            .multipart(reqwest::multipart::Form::new().part(
                "package",
                reqwest::multipart::Part::bytes(package(version)).file_name("fixture-rust.zip"),
            ))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let installed: Value = response.json().await.unwrap();
        assert_eq!(status, 201, "{installed}");
        assert_eq!(installed["id"], "fixture-rust");
        assert_eq!(installed["version"], version);
        assert_eq!(installed["revision"], revision);
        assert_eq!(installed["status"], "disabled");
        assert_eq!(installed["runtime"]["language"], "rust");
        assert_ne!(installed["digest"], previous_digest);
        previous_digest = installed["digest"].clone();
        let response = client
            .post(format!("{base}/fixture-rust/activate"))
            .bearer_auth("background-test-token")
            .header(reqwest::header::IF_MATCH, format!("\"{revision}\""))
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.json::<Value>().await.unwrap()["status"], "ready");
    }
    let response = client
        .post(format!("{base}/install"))
        .bearer_auth("background-test-token")
        .multipart(reqwest::multipart::Form::new().part(
            "package",
            reqwest::multipart::Part::bytes(b"invalid ZIP".to_vec()).file_name("bad.zip"),
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_plugin_package"
    );
    let response: Value = client
        .get(format!("{base}/fixture-rust"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(response["version"], "1.0.1");
    assert_eq!(response["revision"], 4);
    assert_eq!(response["status"], "ready");
    assert_eq!(response["digest"], previous_digest);
    let response: Value = client
        .get(&base)
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(response["total"], 1);
    installer.await.unwrap();
    server.abort();
    let _ = server.await;
    runtime.shutdown().await;
    tokio::fs::remove_file(socket).await.unwrap();
    db.pool().close().await;
    tokio::fs::remove_dir_all(directory).await.unwrap();
}

#[tokio::test]
async fn plugin_upload_rejects_invalid_zip_at_admin_boundary() {
    let (db, config, runtime) = harness().await;
    let mut state = AppState::from_provider_set_with_database(
        (*config).clone(),
        Arc::new(ProviderSet::unavailable()),
        Some((*db).clone()),
    );
    state.background = Some(runtime.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://{}/api/admin/plugins/install",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    let client = reqwest::Client::new();
    for size in [32, 1024 * 1024, 20 * 1024 * 1024] {
        let package = reqwest::multipart::Part::bytes(vec![0; size])
            .file_name("bad.zip")
            .mime_str("application/zip")
            .unwrap();
        let response = client
            .post(&base)
            .bearer_auth("background-test-token")
            .multipart(reqwest::multipart::Form::new().part("package", package))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            400,
            "ZIP size {size} must reach package validation"
        );
        assert_eq!(
            response.json::<Value>().await.unwrap()["error"]["code"],
            "invalid_plugin_package"
        );
    }
    for url in [&base, &base.replace("plugins/install", "jobs")] {
        let length = if url == &base {
            21 * 1024 * 1024 + 1
        } else {
            256 * 1024 + 1
        };
        // The transport rejects the declared length before reading the body.
        // Keep the body pending so an early 413 cannot race a large TCP write.
        let body = reqwest::Body::wrap_stream(futures_util::stream::pending::<
            Result<Vec<u8>, std::io::Error>,
        >());
        let response = client
            .post(url)
            .bearer_auth("background-test-token")
            .header(reqwest::header::CONTENT_LENGTH, length)
            .body(body)
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 413);
        let request_id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let body = response.json::<Value>().await.unwrap();
        assert_eq!(body["error"]["code"], "request_too_large");
        assert_eq!(body["error"]["request_id"], request_id);
    }
    server.abort();
    let _ = server.await;
    runtime.shutdown().await;
}

#[tokio::test]
async fn fixture_zip_reaches_sandbox_preflight_and_fails_closed_on_unqualified_host() {
    let (db, config, runtime) = harness().await;
    let mut state = AppState::from_provider_set_with_database(
        (*config).clone(),
        Arc::new(ProviderSet::unavailable()),
        Some((*db).clone()),
    );
    state.background = Some(runtime.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://{}/api/admin/plugins/install",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/fixture-research-1.0.0.zip"
    ));
    let package = reqwest::multipart::Part::bytes(bytes.to_vec()).file_name("fixture.zip");
    let response = reqwest::Client::new()
        .post(base)
        .bearer_auth("background-test-token")
        .multipart(reqwest::multipart::Form::new().part("package", package))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        "sandbox_unavailable"
    );
    assert_eq!(
        runtime
            .engine
            .plugins
            .list(1, 30, None, None)
            .await
            .unwrap()
            .1,
        0
    );
    server.abort();
    let _ = server.await;
    runtime.shutdown().await;
}

#[tokio::test]
async fn admin_http_tracer_commits_queued_receipt_and_scheduler_observes_it() {
    let (db, config, runtime) = harness().await;
    let mut state = AppState::from_provider_set_with_database(
        (*config).clone(),
        Arc::new(ProviderSet::unavailable()),
        Some((*db).clone()),
    );
    state.background = Some(runtime.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let router = router_with_state(state);
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let url = format!("{base}/jobs");
    let response = client
        .post(&url)
        .bearer_auth("background-test-token")
        .header("Idempotency-Key", "http-once")
        .json(&json!({"goal":"HTTP foundation tracer"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let receipt: Value = response.json().await.unwrap();
    let id = receipt["id"].as_str().unwrap();
    let persisted = runtime.engine.get(id).await.unwrap();
    assert_eq!(persisted.status, "queued");
    wait_for(&runtime.engine, id, |job| job.scheduler_seen_at.is_some()).await;
    let replay = client
        .post(&url)
        .bearer_auth("background-test-token")
        .header("Idempotency-Key", "http-once")
        .json(&json!({"goal":"HTTP foundation tracer"}))
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), 200);
    assert_eq!(replay.json::<Value>().await.unwrap()["id"], id);
    let forged = client
        .post(&url)
        .bearer_auth("background-test-token")
        .json(&json!({"goal":"Forge","agent_id":1}))
        .send()
        .await
        .unwrap();
    assert_eq!(forged.status(), 400);
    let response = client
        .get(&url)
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.json::<Value>().await.unwrap()["total"], 1);
    let response = client
        .post(format!("{url}/{id}/cancel"))
        .bearer_auth("background-test-token")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.json::<Value>().await.unwrap()["status"],
        "cancelled"
    );
    let events_url = format!("{url}/{id}/events?after=0&limit=100");
    assert_eq!(client.get(&events_url).send().await.unwrap().status(), 401);
    let timeline: Value = client
        .get(&events_url)
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(timeline["items"][0]["event_type"], "job_finished");
    assert_eq!(timeline["items"][0]["payload"]["status"], "cancelled");
    let no_more: Value = client
        .get(format!("{url}/{id}/events?after=1"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(no_more["items"].as_array().unwrap().len(), 0);
    let response = client
        .post(format!("{url}/{id}/cancel"))
        .bearer_auth("background-test-token")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
    server.abort();
    let _ = server.await;
    runtime.shutdown().await;
}

#[tokio::test]
async fn job_stream_is_authenticated_and_observes_committed_admin_jobs() {
    use tokio::time::{Duration, timeout};
    let (db, config, runtime) = harness().await;
    let mut state = AppState::from_provider_set_with_database(
        (*config).clone(),
        Arc::new(ProviderSet::unavailable()),
        Some((*db).clone()),
    );
    state.background = Some(runtime.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/api/admin/jobs/stream",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap();
    });
    let client = reqwest::Client::new();
    assert_eq!(client.get(&url).send().await.unwrap().status(), 401);
    let mut response = client
        .get(&url)
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let ready = timeout(Duration::from_secs(3), async {
        let mut text = String::new();
        loop {
            text.push_str(&String::from_utf8_lossy(
                &response.chunk().await.unwrap().unwrap(),
            ));
            if text.contains("event: ready") {
                return text;
            }
        }
    })
    .await
    .unwrap();
    assert!(ready.contains("event: ready"));
    let job = runtime
        .engine
        .submit(
            request("Receive live Job notification"),
            &JobRequester::Admin,
            None,
        )
        .await
        .unwrap()
        .job;
    let observed = timeout(Duration::from_secs(3), async {
        let mut text = String::new();
        loop {
            text.push_str(&String::from_utf8_lossy(
                &response.chunk().await.unwrap().unwrap(),
            ));
            if text.contains("event: job_changed") && text.contains(&job.id) {
                return text;
            }
        }
    })
    .await
    .unwrap();
    assert!(observed.contains(&job.id));
    server.abort();
    runtime.shutdown().await;
}

#[tokio::test]
async fn terminal_job_retention_deletes_durable_events_before_parent() {
    let (db, _config, runtime) = harness().await;
    let job = runtime
        .engine
        .submit(
            request("Retain and purge job timeline"),
            &JobRequester::Admin,
            None,
        )
        .await
        .unwrap()
        .job;
    sqlx::query("UPDATE jobs SET status='cancelled',updated_at=0 WHERE id=? AND status='queued'")
        .bind(&job.id)
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO job_events(job_id,sequence,event_type,created_at,step_key,payload_json) VALUES(?,1,'job_finished',0,NULL,'{}')")
        .bind(&job.id).execute(db.pool()).await.unwrap();
    let expired =
        super::super::artifacts::expire_terminal(&db, crate::database::unix_seconds().unwrap())
            .await
            .unwrap();
    assert_eq!(expired, 1);
    assert!(matches!(
        db.job_view(&job.id).await,
        Err(JobError::NotFound)
    ));
    runtime.shutdown().await;
}

#[tokio::test]
async fn admin_delete_rejects_live_jobs_and_removes_terminal_job_hierarchy() {
    let (db, config, runtime) = harness().await;
    let job = runtime
        .engine
        .submit(request("Delete terminal Job"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    let mut state = AppState::from_provider_set_with_database(
        (*config).clone(),
        Arc::new(ProviderSet::unavailable()),
        Some((*db).clone()),
    );
    state.background = Some(runtime.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://{}/api/admin/jobs/{}",
        listener.local_addr().unwrap(),
        job.id
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    let client = reqwest::Client::new();
    assert_eq!(client.delete(&base).send().await.unwrap().status(), 401);
    let live = client
        .delete(&base)
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(live.status(), 409);
    assert_eq!(
        live.json::<Value>().await.unwrap()["error"]["code"],
        "job_delete_conflict"
    );

    runtime.engine.cancel(&job.id).await.unwrap();
    let root_run = uuid::Uuid::new_v4().to_string();
    let child_run = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO job_agent_runs(agent_run_id,job_id,role,goal_excerpt,allowed_operations_json,status,created_at) VALUES(?,?,'orchestrator','root','[]','succeeded',0)")
        .bind(&root_run).bind(&job.id).execute(db.pool()).await.unwrap();
    sqlx::query("INSERT INTO job_agent_runs(agent_run_id,job_id,parent_agent_run_id,fork_group_id,member_ordinal,role,goal_excerpt,allowed_operations_json,status,created_at) VALUES(?,?,?,'fork',0,'general','child','[]','succeeded',0)")
        .bind(&child_run).bind(&job.id).bind(&root_run).execute(db.pool()).await.unwrap();
    sqlx::query("INSERT INTO job_steps(job_id,step_key,plan_revision,status,created_at,updated_at) VALUES(?,'step',1,'succeeded',0,0)")
        .bind(&job.id).execute(db.pool()).await.unwrap();
    let artifact_id = uuid::Uuid::new_v4().to_string();
    let artifact_dir = super::super::artifacts::root(&db).unwrap().join(&job.id);
    tokio::fs::create_dir_all(&artifact_dir).await.unwrap();
    tokio::fs::write(artifact_dir.join(&artifact_id), b"retired")
        .await
        .unwrap();
    sqlx::query("INSERT INTO job_artifacts(id,job_id,step_key,relative_path,media_type,size_bytes,sha256,created_at) VALUES(?,?,'step',?,'text/plain',7,'test-hash',0)")
        .bind(&artifact_id).bind(&job.id).bind(format!("{}/{artifact_id}",job.id))
        .execute(db.pool()).await.unwrap();

    let cancellation = tokio_util::sync::CancellationToken::new();
    runtime
        .engine
        .cancellations
        .lock()
        .unwrap()
        .insert(job.id.clone(), cancellation.clone());
    let engine = runtime.engine.clone();
    let id = job.id.clone();
    let owner = tokio::spawn(async move {
        cancellation.cancelled().await;
        assert!(
            engine.get(&id).await.is_ok(),
            "retain data until the worker stops"
        );
        engine.cancellations.lock().unwrap().remove(&id);
    });

    assert_eq!(
        client
            .delete(&base)
            .bearer_auth("background-test-token")
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    owner.await.unwrap();
    assert_eq!(
        client
            .get(&base)
            .bearer_auth("background-test-token")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        client
            .get(format!("{base}/events"))
            .bearer_auth("background-test-token")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert!(!artifact_dir.exists());
    assert_eq!(
        client
            .delete(&base)
            .bearer_auth("background-test-token")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    server.abort();
    let _ = server.await;
    runtime.shutdown().await;
}

#[tokio::test]
async fn admin_delete_preserves_retry_when_parent_is_removed() {
    let (db, config, runtime) = harness().await;
    let original = runtime
        .engine
        .submit(request("Retry hierarchy"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    runtime.engine.cancel(&original.id).await.unwrap();
    let retry = runtime
        .engine
        .retry(&original.id, true, Some("retry-lineage-delete"))
        .await
        .unwrap()
        .job;
    let mut state = AppState::from_provider_set_with_database(
        (*config).clone(),
        Arc::new(ProviderSet::unavailable()),
        Some((*db).clone()),
    );
    state.background = Some(runtime.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin/jobs", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    let client = reqwest::Client::new();
    let parent_url = format!("{base}/{}", original.id);
    assert_eq!(
        client
            .delete(&parent_url)
            .bearer_auth("background-test-token")
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    let retained = runtime.engine.get(&retry.id).await.unwrap();
    assert_eq!(retained.status, "queued");
    assert_eq!(retained.retry_of_job_id, None);
    runtime.engine.cancel(&retry.id).await.unwrap();
    assert_eq!(
        client
            .delete(format!("{base}/{}", retry.id))
            .bearer_auth("background-test-token")
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        client
            .delete(&parent_url)
            .bearer_auth("background-test-token")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    server.abort();
    let _ = server.await;
    runtime.shutdown().await;
}

#[tokio::test]
async fn builtin_core_catalog_is_authenticated_and_independent_of_installed_packages() {
    let (db, config, runtime) = harness().await;
    let mut state = AppState::from_provider_set_with_database(
        (*config).clone(),
        Arc::new(ProviderSet::unavailable()),
        Some((*db).clone()),
    );
    state.background = Some(runtime.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://{}/api/admin/plugins",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .get(format!("{base}/builtin"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let catalog: Value = client
        .get(format!("{base}/builtin"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(catalog["total"], 5);
    assert_eq!(catalog["operation_total"], 9);
    let groups = catalog["items"].as_array().unwrap();
    assert!(groups.iter().any(|group| group["id"] == "core.web"));
    for group in groups {
        assert_eq!(group["kind"], "built_in");
        assert_eq!(group["status"], if group["id"] == "core.tts" { "disabled" } else { "ready" });
        assert_eq!(group["runtime"]["kind"], "in_process");
        assert_eq!(group["operations"].as_array().unwrap().len(), if group["id"] == "core.tts" { 1 } else { 2 });
    }
    assert_eq!(
        client
            .get(&base)
            .bearer_auth("background-test-token")
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()["total"],
        0,
        "Core operations must not create Plugin Store installation rows"
    );
    server.abort();
    let _ = server.await;
    runtime.shutdown().await;
}
