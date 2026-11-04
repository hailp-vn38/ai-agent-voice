//! Explicitly authorized, quota-consuming probe. Never part of the default test suite.
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use orchestrator_agent_rs::{OperationUpdate, ToolOperation, ToolOperationStatus};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use crate::database::credentials::CredentialCipher;
use crate::providers::llm::gateway::muse::pool::{SharedMusePool, release_recorded_job_session};
use crate::providers::llm::gateway::muse::proto::SERVICE_DAEMON;
use crate::providers::llm::{
    AgentOutputKind, AgentTaskError, AgentTurnAction, AgentTurnRequest, LlmError, LlmGatewayConfig,
    LlmGatewayProvider, LlmProvider, OperationJournal,
};
use crate::services::llm_gateway::LlmGatewayHub;

struct ProbeJournal {
    operation: Mutex<ToolOperation>,
    path: std::path::PathBuf,
}

#[async_trait::async_trait]
impl OperationJournal for ProbeJournal {
    async fn record(&self, update: OperationUpdate) -> Result<(), LlmError> {
        let mut op = self.operation.lock().unwrap();
        macro_rules! apply {
            ($($field:ident),*) => { $(if let Some(value) = update.$field { op.$field = Some(value); })* };
        }
        apply!(
            provider_ref,
            account_key,
            remote_session_id,
            continuation_key,
            batch_paths_json,
            user_turns_before,
            request_json,
            artifact_ids_json,
            result_json,
            remote_deleted_at
        );
        if let Some(value) = update.status {
            op.status = value;
        }
        if let Some(value) = update.batch_index {
            op.batch_index = value;
        }
        if let Some(value) = update.repairs_used {
            op.repairs_used = value;
        }
        let checkpoint = json!({
            "status": op.status.as_str(), "account_key": op.account_key,
            "remote_session_id": op.remote_session_id,
            "continuation_key": op.continuation_key,
            "baseline": op.user_turns_before, "batch_index": op.batch_index,
            "request_json": op.request_json, "deadline_at": op.deadline_at,
        });
        std::fs::write(&self.path, serde_json::to_vec_pretty(&checkpoint).unwrap())
            .map_err(|_| LlmError::ToolOutcomeUnknown)
    }
}

fn operation(key: &str, account: &str) -> ToolOperation {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let id = uuid::Uuid::new_v4().to_string();
    ToolOperation {
        id: id.clone(),
        invocation_id: id,
        provider_ref: Some("live-stage0".into()),
        account_key: Some(account.into()),
        remote_session_id: None,
        continuation_key: Some(key.into()),
        batch_index: 0,
        repairs_used: 0,
        batch_paths_json: None,
        user_turns_before: None,
        request_json: Some(
            json!({"owner_id":"muse-task-spike",
            "workspace_id":"muse-task-spike", "session_id":key.trim_start_matches("muse_task:"),
            "root_session_id":"muse-task-spike"})
            .to_string(),
        ),
        status: ToolOperationStatus::Submitting,
        artifact_ids_json: None,
        result_json: None,
        remote_deleted_at: None,
        last_activity_at: now,
        deadline_at: now + 900,
        created_at: now,
    }
}

#[tokio::test]
#[ignore = "Requires explicit account authorization; consumes real Muse quota"]
async fn muse_task_live_stage0_probe() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    assert_eq!(
        std::env::var("MUSE_TASK_SPIKE_CONFIRM").as_deref(),
        Ok("spend-quota")
    );
    let root = std::path::PathBuf::from(std::env::var("MUSE_TASK_SPIKE_REPO").unwrap());
    let account = std::env::var("MUSE_TASK_SPIKE_ACCOUNT").unwrap();
    let output_dir = std::path::PathBuf::from(std::env::var("MUSE_TASK_SPIKE_OUTPUT_DIR").unwrap());
    std::fs::create_dir_all(&output_dir).unwrap();
    let cipher = Arc::new(
        CredentialCipher::from_file(&root.join("docker/credential-keys.json"))
            .expect("configured cipher must be available"),
    );
    let options = SqliteConnectOptions::new()
        .filename(root.join("data/vela.db"))
        .read_only(true)
        .busy_timeout(Duration::from_secs(5));
    let db = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let hub = Arc::new(LlmGatewayHub::new(Some(db), cipher));
    hub.reload_snapshot()
        .await
        .expect("read configured account snapshot");
    let config: LlmGatewayConfig = serde_json::from_value(json!({
        "gateway_binding":format!("account:{account}"), "model":"muse-agent",
        "timeout_ms":600000, "timeout_profile":"agentic"
    }))
    .unwrap();
    let provider = LlmGatewayProvider::with_hub(&config, hub.clone()).unwrap();
    let port = provider.agent_task().expect("Muse agent_task adapter");
    let key = format!("muse_task:stage0-{}", uuid::Uuid::new_v4());
    let mut reports = Vec::new();
    let mut previous_session: Option<String> = None;
    let mut last_op = operation(&key, &account);
    let mut failed = false;
    for (index, prompt) in [
        "Research Tokio graceful shutdown using at least two official documentation pages. Browse the pages, summarize the cancellation and task-waiting guidance, include exact source URLs. Write a short report to the requested Markdown file, attach it, then finish. Do not create images or videos.",
        "Continue using the same research context. State the two source URLs you used in the previous turn, then add one paragraph explaining the distinction between cancellation signalling and waiting for tasks. Update and attach the Markdown report. Do not create images or videos.",
    ].into_iter().enumerate() {
        let mut initial = operation(&key, &account);
        initial.remote_session_id = previous_session.clone();
        let journal = ProbeJournal { operation: Mutex::new(initial),
            path: output_dir.join(format!("turn-{}-checkpoint.json", index + 1)) };
        let started = tokio::time::Instant::now();
        let result = loop {
            let request = AgentTurnRequest { session_key: key.clone(),
                action: AgentTurnAction::Send { prompt: prompt.into(),
                    output: AgentOutputKind::Markdown, filename: Some("report.md".into()) },
                operation: journal.operation.lock().unwrap().clone(),
                wait_slice: Duration::from_secs(1700) };
            match port.run_turn(&request, &journal, &|_| {}).await {
                Err(AgentTaskError::RetryLater(_)) if started.elapsed() < Duration::from_secs(840) => {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                result => break result,
            }
        };
        last_op = journal.operation.lock().unwrap().clone();
        let mut report = json!({"turn":index+1, "elapsed_secs":started.elapsed().as_secs(),
            "session_id":last_op.remote_session_id, "baseline":last_op.user_turns_before,
            "status":last_op.status.as_str()});
        match result {
            Ok(result) => {
                report["result"] = json!("PASS");
                report["message"] = json!(result.message);
                report["reported_turn"] = json!(result.turn);
                report["files_missing"] = json!(result.files_missing);
                let files: Vec<Value> = result.files.iter().enumerate().map(|(n, file)| {
                    // Remote filenames never become local paths.
                    let destination = output_dir.join(format!("turn-{}-file-{n}.bin", index+1));
                    std::fs::write(destination, &file.bytes).unwrap();
                    json!({"filename":file.filename,"mime_type":file.mime_type,
                        "source_path":file.source_path,"size":file.bytes.len(),
                        "sha256":Sha256::digest(&file.bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>()})
                }).collect();
                report["files"] = json!(files);
            }
            Err(error) => { report["result"] = json!("FAIL");
                report["error"] = json!(error.to_string()); failed = true; }
        }
        previous_session = last_op.remote_session_id.clone();
        reports.push(report);
        std::fs::write(output_dir.join("report.json"), serde_json::to_vec_pretty(&reports).unwrap()).unwrap();
        if failed { break; }
    }
    // Inspect only metadata/event types, never credential-bearing requests.
    let mut history_evidence = json!({"result":"NOT_RUN"});
    if let Some(sid) = &last_op.remote_session_id
        && let Ok(lease) = hub.acquire_account_for_polling(&account)
    {
        let pool = SharedMusePool::for_gateway_lease(
            &reqwest::Client::new(),
            &account,
            lease.access_token.expose(),
            None,
        );
        if let Ok(mut vm) = pool.acquire_existing_session(sid.clone()).await {
            vm.retain_session();
            if let Ok(response) = vm
                .client()
                .send_request(
                    "GET",
                    &format!("/chat/history?session_id={sid}"),
                    None,
                    SERVICE_DAEMON,
                    Duration::from_secs(20),
                    false,
                )
                .await
            {
                let value = response.json().unwrap_or(Value::Null);
                std::fs::write(
                    output_dir.join("history.json"),
                    serde_json::to_vec_pretty(&value).unwrap(),
                )
                .unwrap();
                history_evidence = json!({"result":"PASS","http_status":response.status});
            }
            vm.finish_clean(None);
        }
    }
    let deleted = release_recorded_job_session(&last_op).await.is_some();
    let summary = json!({"turns":reports,"history":history_evidence,
        "remote_deleted_ack":deleted,"same_session":reports.len()==2 && reports[0]["session_id"]==reports[1]["session_id"]});
    std::fs::write(
        output_dir.join("report.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();
    assert!(
        !failed && deleted,
        "Live task probe failed; inspect saved report (no credentials)"
    );
}
