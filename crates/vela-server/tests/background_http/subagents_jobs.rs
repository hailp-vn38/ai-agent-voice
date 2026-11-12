use super::harness::*;
use crate::admin_auth::admin_web_token;

#[tokio::test]
async fn background_custom_subagents_crud_flow_isolation_and_concurrency() {
    let temp_root =
        std::env::temp_dir().join(format!("background-subagents-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_root).unwrap();
    let prompts_dir = temp_root.join("prompts");
    let config: AppConfig = toml::from_str(&format!(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "sqlite://{}"
[api]
enabled = true
jwt_secret = "background-test-jwt-signing-secret-0123456789"
[orchestrator]
store_dir = "{}"
prompts_dir = "{}"
skills_dir = "{}"
"#,
        temp_root.join("vela.db").display(),
        temp_root.join("orch-store").display(),
        prompts_dir.display(),
        temp_root.join("skills").display(),
    ))
    .unwrap();
    let router = bootstrap_with_providers(config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let web_token = admin_web_token(&reqwest::Client::new(), base.trim_end_matches("/api/admin")).await;
    let client = Client::new();

    // 1. Initial settings has 10 built-in agents with `is_builtin: true`
    let initial: Value = client
        .get(format!("{base}/background/settings"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(initial["revision"], 1);
    let initial_agents = initial["agents"].as_array().unwrap();
    assert_eq!(initial_agents.len(), 11);
    assert!(initial_agents.iter().all(|a| a["is_builtin"] == true));

    // 2. Create a Custom Sub-Agent in Flow 1 (`daily`) via POST /api/admin/background/agents
    let create_resp = client
        .post(format!("{base}/background/agents"))
        .bearer_auth(&web_token)
        .header("If-Match", "\"1\"")
        .json(&json!({
            "id": "custom_arxiv_digest",
            "flow_id": "daily",
            "enabled": true,
            "steps": 18,
            "allowed_tools": ["fetch_html", "markdown_create"],
            "allowed_skills": ["wiki-query"],
            "description": "Summarizes arXiv papers into Markdown briefs.",
            "system_prompt": "You are the custom arXiv digest subagent."
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let after_create: Value = create_resp.json().await.unwrap();
    assert_eq!(after_create["revision"], 2);
    let agents = after_create["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 12);
    let custom = agents
        .iter()
        .find(|a| a["id"] == "custom_arxiv_digest")
        .expect("custom subagent present");
    assert_eq!(custom["is_builtin"], false);
    assert_eq!(custom["flow_id"], "daily");
    assert_eq!(custom["parent_id"], "daily_root");
    assert_eq!(custom["enabled"], true);
    assert_eq!(custom["steps"], 18);
    assert_eq!(custom["default_steps"], 18);
    assert_eq!(
        custom["allowed_tools"],
        json!(["skill", "fetch_html", "markdown_create"])
    );
    assert_eq!(custom["allowed_skills"], json!(["wiki-query"]));
    assert_eq!(custom["default_allowed_skills"], json!(["wiki-query"]));
    assert_eq!(custom["required_job_skills"], json!([]));

    let daily_root = agents.iter().find(|a| a["id"] == "daily_root").unwrap();
    assert!(
        daily_root["allowed_subagents"]
            .as_array()
            .unwrap()
            .contains(&json!("custom_arxiv_digest"))
    );
    assert!(prompts_dir.join("custom_arxiv_digest.md").exists());
    let daily_root_md = std::fs::read_to_string(prompts_dir.join("daily_root.md")).unwrap();
    assert!(daily_root_md.contains("`custom_arxiv_digest` (custom)"));

    // 3. Reject duplicate ID, built-in ID collision, invalid flow, or Flow 2 tool violation
    let dup_resp = client
        .post(format!("{base}/background/agents"))
        .bearer_auth(&web_token)
        .header("If-Match", "\"2\"")
        .json(&json!({
            "id": "custom_arxiv_digest",
            "flow_id": "daily",
            "description": "Dup",
            "system_prompt": "Dup"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(dup_resp.status(), StatusCode::CONFLICT);
    assert_eq!(
        dup_resp.json::<Value>().await.unwrap()["error"]["code"],
        "agent_already_exists"
    );

    let builtin_dup = client
        .post(format!("{base}/background/agents"))
        .bearer_auth(&web_token)
        .header("If-Match", "\"2\"")
        .json(&json!({
            "id": "source_collector",
            "flow_id": "daily",
            "description": "Dup builtin",
            "system_prompt": "Dup"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(builtin_dup.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        builtin_dup.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_agent_id"
    );

    let bad_wiki_tool = client
        .post(format!("{base}/background/agents"))
        .bearer_auth(&web_token)
        .header("If-Match", "\"2\"")
        .json(&json!({
            "id": "wiki_bad_collector",
            "flow_id": "wiki",
            "allowed_tools": ["yt_fetch_transcript"],
            "description": "Tries to use YouTube tool in Flow 2",
            "system_prompt": "Invalid tool for Flow 2"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_wiki_tool.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        bad_wiki_tool.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_arguments"
    );

    // 4. Partial toggle via PUT /api/admin/background/settings preserves flow_id & prompt
    let toggle_resp = client
        .put(format!("{base}/background/settings"))
        .bearer_auth(&web_token)
        .header("If-Match", "\"2\"")
        .json(&json!({
            "enabled": true,
            "agents": {
                "custom_arxiv_digest": {
                    "enabled": false
                }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(toggle_resp.status(), StatusCode::OK);
    let toggled: Value = toggle_resp.json().await.unwrap();
    assert_eq!(toggled["revision"], 3);
    let toggled_agents = toggled["agents"].as_array().unwrap();
    let toggled_custom = toggled_agents
        .iter()
        .find(|a| a["id"] == "custom_arxiv_digest")
        .unwrap();
    assert_eq!(toggled_custom["enabled"], false);
    assert_eq!(
        toggled_custom["description"],
        "Summarizes arXiv papers into Markdown briefs."
    );
    let toggled_daily_root = toggled_agents
        .iter()
        .find(|a| a["id"] == "daily_root")
        .unwrap();
    assert!(
        !toggled_daily_root["allowed_subagents"]
            .as_array()
            .unwrap()
            .contains(&json!("custom_arxiv_digest"))
    );

    // 5. Deleting a Built-in Sub-Agent is rejected with 400 builtin_agent_cannot_be_deleted
    let del_builtin = client
        .delete(format!("{base}/background/agents/source_collector"))
        .bearer_auth(&web_token)
        .header("If-Match", "\"3\"")
        .send()
        .await
        .unwrap();
    assert_eq!(del_builtin.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        del_builtin.json::<Value>().await.unwrap()["error"]["code"],
        "builtin_agent_cannot_be_deleted"
    );

    // 6. Delete Custom Sub-Agent with If-Match -> 200 OK, removes agent & prompt file
    let del_custom = client
        .delete(format!("{base}/background/agents/custom_arxiv_digest"))
        .bearer_auth(&web_token)
        .header("If-Match", "\"3\"")
        .send()
        .await
        .unwrap();
    assert_eq!(del_custom.status(), StatusCode::OK);
    let after_del: Value = del_custom.json().await.unwrap();
    assert_eq!(after_del["revision"], 4);
    assert_eq!(after_del["agents"].as_array().unwrap().len(), 11);
    assert!(!prompts_dir.join("custom_arxiv_digest.md").exists());

    server.abort();
}

#[tokio::test]
async fn post_jobs_accepts_wav_attachment_over_256kib_validates_idempotency_and_preserves_route_body_caps()
 {
    use base64::Engine as _;

    let temp_root =
        std::env::temp_dir().join(format!("background-jobs-wav-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_root).unwrap();
    let config: AppConfig = toml::from_str(&format!(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "sqlite://{}"
[api]
enabled = true
jwt_secret = "background-test-jwt-signing-secret-0123456789"
[orchestrator]
enabled = true
endpoint = "http://127.0.0.1:9/v1"
api_key = "test-key"
model = "test-model"
store_dir = "{}"
tasks_store_dir = "{}"
reports_dir = "{}"
work_dir = "{}"
prompts_dir = "{}"
skills_dir = "{}"
[wiki]
vault_dir = "{}"
"#,
        temp_root.join("vela.db").display(),
        temp_root.join("orch-store").display(),
        temp_root.join("tasks-store").display(),
        temp_root.join("reports").display(),
        temp_root.join("work").display(),
        temp_root.join("prompts").display(),
        temp_root.join("skills").display(),
        temp_root.join("wiki").display(),
    ))
    .unwrap();
    let router = bootstrap_with_providers(config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let web_token = admin_web_token(&reqwest::Client::new(), base.trim_end_matches("/api/admin")).await;
    let client = Client::new();

    // Helper to build a valid canonical 16 kHz mono 16-bit PCM WAV with `sample_count` samples
    let make_wav = |sample_count: usize, seed: i16| -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
        for i in 0..sample_count {
            writer
                .write_sample(seed.wrapping_add((i % 97) as i16))
                .unwrap();
        }
        writer.finalize().unwrap();
        cursor.into_inner()
    };

    // 1. Valid `.wav` attachment whose base64 body exceeds 256 KiB (e.g. 160,000 samples = 320 KiB PCM -> ~427 KiB base64)
    let wav_a = make_wav(160_000, 100);
    let wav_a_b64 = base64::engine::general_purpose::STANDARD.encode(&wav_a);
    assert!(wav_a_b64.len() > 256 * 1024);

    let created_resp = client
        .post(format!("{base}/jobs"))
        .bearer_auth(&web_token)
        .header("Idempotency-Key", "idem-wav-1")
        .json(&json!({
            "goal": "Transcribe this meeting recording",
            "attachments": [{
                "name": "meeting.wav",
                "mime_type": "audio/wav",
                "data_base64": wav_a_b64,
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created_resp.status(), StatusCode::CREATED);
    let created_job: Value = created_resp.json().await.unwrap();
    let job_id = created_job["id"].as_str().unwrap().to_owned();
    let artifacts = created_job["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0]["name"], "meeting.wav");
    assert_eq!(artifacts[0]["kind"], "audio");
    assert_eq!(artifacts[0]["role"], "intermediate");
    assert!(artifacts[0]["producer_invocation_id"].is_null());

    // 1b. Valid `.mp4` video attachment -> 201 Created, classified as `kind == "video"`, `role == "intermediate"`
    let mp4_bytes = {
        let mut out = Vec::with_capacity(128);
        // Box 1: `ftyp` (24 bytes)
        out.extend_from_slice(&24u32.to_be_bytes());
        out.extend_from_slice(b"ftypisom");
        out.extend_from_slice(&512u32.to_be_bytes());
        out.extend_from_slice(b"isommp42");
        // Box 2: `moov` (40 bytes) with `mvhd` (32 bytes)
        out.extend_from_slice(&40u32.to_be_bytes());
        out.extend_from_slice(b"moov");
        out.extend_from_slice(&32u32.to_be_bytes());
        out.extend_from_slice(b"mvhd");
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&1000u32.to_be_bytes());
        out.extend_from_slice(&3000u32.to_be_bytes());
        // Box 3: `mdat`
        let payload = b"MP4_STREAM|640x360|3s|prompt=demo";
        let mdat_len = (8 + payload.len()) as u32;
        out.extend_from_slice(&mdat_len.to_be_bytes());
        out.extend_from_slice(b"mdat");
        out.extend_from_slice(payload);
        out
    };
    let mp4_b64 = base64::engine::general_purpose::STANDARD.encode(&mp4_bytes);
    let mp4_resp = client
        .post(format!("{base}/jobs"))
        .bearer_auth(&web_token)
        .header("Idempotency-Key", "idem-mp4-1")
        .json(&json!({
            "goal": "Transcribe and summarize this attached MP4 video",
            "attachments": [{
                "name": "demo.mp4",
                "mime_type": "video/mp4",
                "data_base64": mp4_b64,
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(mp4_resp.status(), StatusCode::CREATED);
    let mp4_job: Value = mp4_resp.json().await.unwrap();
    let mp4_artifacts = mp4_job["artifacts"].as_array().unwrap();
    assert_eq!(mp4_artifacts.len(), 1);
    assert_eq!(mp4_artifacts[0]["name"], "demo.mp4");
    assert_eq!(mp4_artifacts[0]["kind"], "video");
    assert_eq!(mp4_artifacts[0]["role"], "intermediate");
    assert!(mp4_artifacts[0]["producer_invocation_id"].is_null());

    // 2. Same `Idempotency-Key` with a DIFFERENT `.wav` file -> 409 idempotency_key_conflict
    let wav_b = make_wav(16_000, 200);
    let wav_b_b64 = base64::engine::general_purpose::STANDARD.encode(&wav_b);
    let idem_conflict = client
        .post(format!("{base}/jobs"))
        .bearer_auth(&web_token)
        .header("Idempotency-Key", "idem-wav-1")
        .json(&json!({
            "goal": "Transcribe this meeting recording",
            "attachments": [{
                "name": "meeting.wav",
                "mime_type": "audio/wav",
                "data_base64": wav_b_b64,
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(idem_conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        idem_conflict.json::<Value>().await.unwrap()["error"]["code"],
        "idempotency_key_conflict"
    );

    // 3. Non-`.wav`/`.mp4` attachment -> 400 invalid_attachment
    let bad_file = client
        .post(format!("{base}/jobs"))
        .bearer_auth(&web_token)
        .json(&json!({
            "goal": "Analyze file",
            "attachments": [{
                "name": "notes.txt",
                "mime_type": "text/plain",
                "data_base64": base64::engine::general_purpose::STANDARD.encode(b"not a wav"),
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_file.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        bad_file.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_attachment"
    );

    // 4. File > 25 MB -> 413 attachment_too_large
    let oversized_bytes = vec![b'A'; 25 * 1024 * 1024 + 64];
    let oversized_b64 = base64::engine::general_purpose::STANDARD.encode(&oversized_bytes);
    let too_large = client
        .post(format!("{base}/jobs"))
        .bearer_auth(&web_token)
        .json(&json!({
            "goal": "Oversized wav",
            "attachments": [{
                "name": "huge.wav",
                "mime_type": "audio/wav",
                "data_base64": oversized_b64,
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        too_large.json::<Value>().await.unwrap()["error"]["code"],
        "attachment_too_large"
    );

    // 5. Sub-routes `/jobs/{id}/*` still enforce the 256 KiB `MAX_BODY` limit -> 413 payload_too_large
    let huge_followup = "x".repeat(300 * 1024);
    let followup_resp = client
        .post(format!("{base}/jobs/{job_id}/followup"))
        .bearer_auth(&web_token)
        .json(&json!({ "input": huge_followup }))
        .send()
        .await
        .unwrap();
    assert_eq!(followup_resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        followup_resp.json::<Value>().await.unwrap()["error"]["code"],
        "request_too_large"
    );

    server.abort();
}

#[tokio::test]
async fn admin_http_batch_delete_jobs_removes_selected_tasks_and_validates_payload() {
    let temp_root =
        std::env::temp_dir().join(format!("background-batch-del-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_root).unwrap();
    let config: AppConfig = toml::from_str(&format!(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "sqlite://{}"
[api]
enabled = true
jwt_secret = "background-test-jwt-signing-secret-0123456789"
[orchestrator]
enabled = true
endpoint = "http://127.0.0.1:9/v1"
api_key = "test-key"
model = "test-model"
store_dir = "{}"
tasks_store_dir = "{}"
reports_dir = "{}"
work_dir = "{}"
prompts_dir = "{}"
skills_dir = "{}"
[wiki]
vault_dir = "{}"
"#,
        temp_root.join("vela.db").display(),
        temp_root.join("orch-store").display(),
        temp_root.join("tasks-store").display(),
        temp_root.join("reports").display(),
        temp_root.join("work").display(),
        temp_root.join("prompts").display(),
        temp_root.join("skills").display(),
        temp_root.join("wiki").display(),
    ))
    .unwrap();
    let router = bootstrap_with_providers(config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let web_token = admin_web_token(&reqwest::Client::new(), base.trim_end_matches("/api/admin")).await;
    let client = Client::new();

    // 1. Unauthenticated request is rejected
    let unauth = client
        .post(format!("{base}/jobs/batch-delete"))
        .json(&json!({ "ids": ["job-1"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

    // 2. Empty or blank IDs are rejected with 400 invalid_arguments
    for bad_payload in [json!({ "ids": [] }), json!({ "ids": ["   "] })] {
        let bad_resp = client
            .post(format!("{base}/jobs/batch-delete"))
            .bearer_auth(&web_token)
            .json(&bad_payload)
            .send()
            .await
            .unwrap();
        assert_eq!(bad_resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            bad_resp.json::<Value>().await.unwrap()["error"]["code"],
            "invalid_arguments"
        );
    }

    // 3. Create 3 jobs and batch-delete 2 of them (with duplicate + missing IDs)
    let mut created_ids = Vec::new();
    for idx in 1..=3 {
        let resp = client
            .post(format!("{base}/jobs"))
            .bearer_auth(&web_token)
            .json(&json!({ "goal": format!("Batch delete test job #{idx}") }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body: Value = resp.json().await.unwrap();
        created_ids.push(body["id"].as_str().unwrap().to_owned());
    }

    let del_resp = client
        .post(format!("{base}/jobs/batch-delete"))
        .bearer_auth(&web_token)
        .json(&json!({
            "ids": [
                &created_ids[0],
                &created_ids[1],
                &created_ids[0],
                "missing-job-id"
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), StatusCode::OK);
    let del_body: Value = del_resp.json().await.unwrap();
    assert_eq!(del_body["deleted"], 2);
    assert_eq!(
        del_body["deleted_ids"],
        json!([&created_ids[0], &created_ids[1]])
    );

    // 4. Verify deleted jobs return 404 and remaining job stays in GET /api/admin/jobs
    for deleted_id in [&created_ids[0], &created_ids[1]] {
        let get_resp = client
            .get(format!("{base}/jobs/{deleted_id}"))
            .bearer_auth(&web_token)
            .send()
            .await
            .unwrap();
        assert_eq!(get_resp.status(), StatusCode::NOT_FOUND);
    }

    let list_resp = client
        .get(format!("{base}/jobs"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), StatusCode::OK);
    let list_body: Value = list_resp.json().await.unwrap();
    assert_eq!(list_body["total"], 1);
    assert_eq!(list_body["items"][0]["id"], created_ids[2]);

    server.abort();
}

#[tokio::test]
async fn batch_durability_routes_are_authenticated_and_fail_closed_without_a_task_manager() {
    let temp_root = std::env::temp_dir().join(format!("background-http-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_root).unwrap();
    let config: AppConfig = toml::from_str(&format!(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "sqlite://{}"
[api]
enabled = true
jwt_secret = "background-test-jwt-signing-secret-0123456789"
[orchestrator]
store_dir = "{}"
prompts_dir = "{}"
"#,
        temp_root.join("vela.db").display(),
        temp_root.join("orch-store").display(),
        temp_root.join("prompts").display(),
    ))
    .unwrap();
    let router = bootstrap_with_providers(config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let web_token = admin_web_token(&reqwest::Client::new(), base.trim_end_matches("/api/admin")).await;
    let client = Client::new();

    let quarantine = format!("{base}/knowledge/quarantined-sources");
    let release = format!("{quarantine}/release");
    let resolve = format!("{base}/jobs/job-1/invocations/inv-1/resolve");
    let verdict = json!({ "decision": "no_effect", "evidence": "checked" });

    assert_eq!(
        client.get(&quarantine).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        client
            .post(&resolve)
            .json(&verdict)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );

    let token = web_token.as_str();
    assert_eq!(
        client
            .get(&quarantine)
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        client
            .post(&release)
            .bearer_auth(token)
            .json(&json!({ "source_rel_path": "sources/a.md" }))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        client
            .post(&resolve)
            .bearer_auth(token)
            .json(&verdict)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        client
            .post(&resolve)
            .bearer_auth(token)
            .json(&json!({ "decision": "guess" }))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST,
        "an unknown verdict is rejected before anything else"
    );

    server.abort();
}
