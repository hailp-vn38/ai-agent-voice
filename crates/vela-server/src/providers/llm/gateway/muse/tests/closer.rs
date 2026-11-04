//! Lifecycle acceptance through the real Orchestrator, host tool, and a scripted VM.
use super::super::pool::SharedMusePool;
use super::harness::{
    FakeMuseVmTransport, FakeVmAccount, SINGLE_COOKIE, build_gateway_muse_provider,
};
use crate::providers::llm::{
    AgentTaskError, AgentTaskProvider, AgentTurnOutput, AgentTurnRequest, OperationJournal,
};
use crate::services::llm_gateway::{GatewayBinding, GatewayVendor};
use crate::services::task_flow::muse_task::{
    MuseTaskTool,
    closer::SubagentMuseCloser,
    tests::{ScriptedModel, scripted_provider, temp_root},
};
use async_trait::async_trait;
use orchestrator_agent_rs::{
    AgentDefinition, AgentMode, CallerContext, LlmConfig, OperationUpdate, Orchestrator,
    OrchestratorConfig, RunStatus, ToolOperationStatus,
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};

struct RecordedRemote {
    account: String,
    account_override: std::sync::Mutex<Option<String>>,
    same_remote: AtomicBool,
    restored_requests: std::sync::Mutex<Vec<Option<String>>>,
    calls: AtomicU32,
    block_at: AtomicU32,
    corrupt_at: AtomicU32,
    gate: tokio::sync::Semaphore,
}
#[async_trait]
impl AgentTaskProvider for RecordedRemote {
    async fn run_turn(
        &self,
        _request: &AgentTurnRequest,
        journal: &dyn OperationJournal,
        _progress: &(dyn for<'a> Fn(&'a str) + Send + Sync),
    ) -> Result<AgentTurnOutput, AgentTaskError> {
        self.restored_requests
            .lock()
            .unwrap()
            .push(_request.operation.remote_session_id.clone());
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let account = self
            .account_override
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| self.account.clone());
        journal
            .record(OperationUpdate {
                account_key: Some(account),
                remote_session_id: Some(format!(
                    "closer-remote-{}",
                    if self.same_remote.load(Ordering::SeqCst) {
                        1
                    } else {
                        call
                    }
                )),
                status: Some(ToolOperationStatus::Running),
                request_json: (self.corrupt_at.load(Ordering::SeqCst) == call).then(|| "{}".into()),
                ..Default::default()
            })
            .await
            .map_err(|_| AgentTaskError::OutcomeUnknown)?;
        if self.block_at.load(Ordering::SeqCst) == call {
            let _permit = self.gate.acquire().await.unwrap();
        }
        Ok(AgentTurnOutput {
            turn: call,
            message: "Durable report".into(),
            files: vec![],
            files_missing: false,
        })
    }
}

struct Lifecycle {
    dir: PathBuf,
    orch: Arc<Orchestrator>,
    model: Arc<ScriptedModel>,
    vm: Arc<FakeVmAccount>,
    _pool: Arc<SharedMusePool>,
    caller: CallerContext,
    root: String,
    provider: Arc<RecordedRemote>,
    config: OrchestratorConfig,
}
impl Lifecycle {
    async fn boot() -> Self {
        let dir = temp_root("muse-closer");
        let model = ScriptedModel::serve().await;
        let vm = FakeVmAccount::new(vec![]);
        let transport = FakeMuseVmTransport::new(vec![vm.clone()]);
        let gateway = build_gateway_muse_provider(&[SINGLE_COOKIE], transport.clone()).await;
        let lease = gateway
            .hub
            .acquire_lease(&GatewayBinding::Pool(GatewayVendor::Muse))
            .unwrap();
        let pool = SharedMusePool::for_gateway_lease(
            &reqwest::Client::new(),
            &lease.account_key,
            lease.access_token.expose(),
            Some(transport),
        );
        let prompt = dir.join("test.md");
        std::fs::write(&prompt, "Test assistant").unwrap();
        let primary = AgentDefinition::new("root", "test", "test-model")
            .with_mode(AgentMode::Primary)
            .with_prompt_path(&prompt)
            .with_allowed_tools(["subagent"])
            .with_allowed_subagents(["cloud"]);
        let child = AgentDefinition::new("cloud", "test", "test-model")
            .with_mode(AgentMode::Subagent)
            .with_description("Cloud worker")
            .with_prompt_path(&prompt)
            .with_allowed_tools(["muse_task"]);
        let mut config = OrchestratorConfig::new(
            LlmConfig {
                providers: vec![scripted_provider("test", &model)],
            },
            vec![primary, child],
            dir.join("orch"),
        );
        // Force pagination even with two attempts/turns.
        config.runtime.page_size = 1;
        let provider = Arc::new(RecordedRemote {
            account: lease.account_key.clone(),
            calls: AtomicU32::new(0),
            restored_requests: std::sync::Mutex::new(Vec::new()),
            account_override: std::sync::Mutex::new(None),
            same_remote: AtomicBool::new(false),
            block_at: AtomicU32::new(0),
            corrupt_at: AtomicU32::new(0),
            gate: tokio::sync::Semaphore::new(0),
        });
        let orch = Arc::new(
            Orchestrator::builder(config.clone())
                .register_tool(Arc::new(MuseTaskTool::for_provider(
                    provider.clone(),
                    128 * 1024 * 1024,
                    24 * 1024,
                )))
                .build()
                .await
                .unwrap(),
        );
        let caller = CallerContext::new("owner", "workspace");
        let root = orch.create_session(&caller, "root").await.unwrap();
        Self {
            dir,
            orch,
            model,
            vm,
            _pool: pool,
            caller,
            root,
            provider,
            config,
        }
    }
    async fn start_child(&self, existing: Option<&str>) -> (String, String) {
        self.start_child_with(
            existing,
            json!({"prompt":"Research", "output":"markdown", "delete_session_on_finish":true}),
        )
        .await
    }
    async fn start_child_with(&self, existing: Option<&str>, args: Value) -> (String, String) {
        self.model.queue("subagent", json!({"agent":"cloud", "description":"Report", "prompt":"Research", "session_id":existing}));
        let call = self.model.queue("muse_task", args);
        let turn = self
            .orch
            .submit(&self.caller, &self.root, "Research".into())
            .await
            .unwrap();
        (call, turn.turn_id)
    }
    async fn wait_root(&self, turn: &str) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let result = self
                    .orch
                    .result(&self.caller, &self.root, turn)
                    .await
                    .unwrap();
                if result.status == RunStatus::Completed {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
    async fn complete_child(&self) -> Value {
        let (call, turn) = self.start_child(None).await;
        self.wait_root(&turn).await;
        serde_json::from_str(
            &self
                .model
                .tool_result(&call)
                .expect("tool result reaches model"),
        )
        .unwrap()
    }
    async fn child_session(&self) -> String {
        let operations = self.orch.list_tool_operations().await.unwrap();
        let journal: Value =
            serde_json::from_str(operations[0].operation.request_json.as_deref().unwrap()).unwrap();
        journal["session_id"].as_str().unwrap().to_owned()
    }
    async fn finish(self) {
        self.orch.shutdown().await.unwrap();
        let _ = std::fs::remove_dir_all(self.dir);
    }
}

#[tokio::test]
async fn muse_task_closer_archives_before_delete_and_parent_keeps_artifact() {
    let lifecycle = Lifecycle::boot().await;
    let result = lifecycle.complete_child().await;
    let before = lifecycle.orch.list_tool_operations().await.unwrap();
    let op = &before[0].operation;
    let journal: Value = serde_json::from_str(op.request_json.as_deref().unwrap()).unwrap();
    let child = journal["session_id"].as_str().unwrap();
    assert!(lifecycle.vm.deleted_sessions().is_empty());
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    lifecycle.vm.gate_delete(gate.clone());
    let orch = lifecycle.orch.clone();
    let sweep = tokio::spawn(async move {
        SubagentMuseCloser::sweep(&orch).await.unwrap();
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        while lifecycle.vm.delete_attempts().is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    // Remote has received DELETE but has not acknowledged it yet.
    assert!(lifecycle.vm.deleted_sessions().is_empty());
    assert!(
        lifecycle.orch.list_tool_operations().await.unwrap()[0]
            .operation
            .remote_deleted_at
            .is_none()
    );
    // The public family-artifact API is the parent's read boundary after child archive.
    let id = result["artifact_ids"][0].as_str().unwrap();
    let artifact = lifecycle
        .orch
        .open_family_artifact(&lifecycle.caller, &lifecycle.root, id)
        .await
        .unwrap();
    assert_eq!(
        artifact.into_reader().read_to_string().await.unwrap(),
        "Durable report"
    );
    let call = lifecycle.model.queue(
        "subagent",
        json!({"agent":"cloud", "description":"Continue", "prompt":"Continue", "session_id":child}),
    );
    lifecycle
        .orch
        .submit(&lifecycle.caller, &lifecycle.root, "Continue".into())
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(output) = lifecycle.model.tool_result(&call) {
                break output;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(response.contains("SESSION_ARCHIVED"), "{response}");
    gate.add_permits(1);
    sweep.await.unwrap();
    assert_eq!(lifecycle.vm.deleted_sessions(), ["closer-remote-1"]);
    assert!(
        lifecycle.orch.list_tool_operations().await.unwrap()[0]
            .operation
            .remote_deleted_at
            .is_some()
    );
    lifecycle.finish().await;
}

#[tokio::test]
async fn muse_task_closer_keeps_remote_session_by_default_after_archiving_child() {
    let lifecycle = Lifecycle::boot().await;
    let (_call, turn) = lifecycle
        .start_child_with(None, json!({"prompt":"Research", "output":"markdown"}))
        .await;
    lifecycle.wait_root(&turn).await;
    let child = lifecycle.child_session().await;
    let ops = lifecycle.orch.list_tool_operations().await.unwrap();
    super::super::pool::restore_job_sessions_from_operations(&ops).await;
    let entry = super::super::pool::job_entry(&format!("muse_task:{child}"));
    assert!(entry.lock().await.is_some());

    SubagentMuseCloser::sweep(&lifecycle.orch).await.unwrap();
    SubagentMuseCloser::sweep(&lifecycle.orch).await.unwrap();

    assert!(lifecycle.vm.delete_attempts().is_empty());
    assert!(
        lifecycle.orch.list_tool_operations().await.unwrap()[0]
            .operation
            .remote_deleted_at
            .is_none()
    );
    assert!(
        lifecycle
            .orch
            .get_session(&lifecycle.caller, &child)
            .await
            .unwrap()
            .archived
    );
    // The child no longer holds the session once it has finished.
    assert!(entry.lock().await.is_none());
    assert!(entry.account_hint().is_none());
    lifecycle.finish().await;
}

#[tokio::test]
async fn muse_task_closer_delete_request_on_any_turn_covers_the_whole_child() {
    let lifecycle = Lifecycle::boot().await;
    let (_call, turn) = lifecycle
        .start_child_with(None, json!({"prompt":"Research", "output":"markdown"}))
        .await;
    lifecycle.wait_root(&turn).await;
    let child = lifecycle.child_session().await;
    let (_call, turn) = lifecycle.start_child(Some(&child)).await;
    lifecycle.wait_root(&turn).await;

    SubagentMuseCloser::sweep(&lifecycle.orch).await.unwrap();

    let mut deleted = lifecycle.vm.deleted_sessions();
    deleted.sort();
    assert_eq!(deleted, ["closer-remote-1", "closer-remote-2"]);
    lifecycle.finish().await;
}

#[tokio::test]
async fn muse_task_closer_retries_delete_until_remote_acknowledges() {
    let lifecycle = Lifecycle::boot().await;
    lifecycle.complete_child().await;
    lifecycle.vm.set_delete_failure_status(Some(503));
    SubagentMuseCloser::sweep(&lifecycle.orch).await.unwrap();
    assert!(lifecycle.vm.deleted_sessions().is_empty());
    assert_eq!(lifecycle.vm.delete_attempts(), ["closer-remote-1"]);
    assert!(
        lifecycle.orch.list_tool_operations().await.unwrap()[0]
            .operation
            .remote_deleted_at
            .is_none()
    );
    lifecycle.vm.set_delete_failure_status(None);
    SubagentMuseCloser::sweep(&lifecycle.orch).await.unwrap();
    assert_eq!(
        lifecycle.vm.delete_attempts(),
        ["closer-remote-1", "closer-remote-1"]
    );
    assert!(
        lifecycle.orch.list_tool_operations().await.unwrap()[0]
            .operation
            .remote_deleted_at
            .is_some()
    );
    // Once acknowledged, future passes perform no further remote work.
    SubagentMuseCloser::sweep(&lifecycle.orch).await.unwrap();
    assert_eq!(lifecycle.vm.delete_attempts().len(), 2);
    lifecycle.finish().await;
}

#[tokio::test]
async fn muse_task_closer_reads_latest_paginated_run_and_releases_every_remote_pair() {
    let lifecycle = Lifecycle::boot().await;
    lifecycle.complete_child().await;
    let child = lifecycle.child_session().await;
    lifecycle.provider.block_at.store(2, Ordering::SeqCst);
    let (_call, turn) = lifecycle.start_child(Some(&child)).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let ops = lifecycle.orch.list_tool_operations().await.unwrap();
            if ops
                .iter()
                .any(|s| s.operation.remote_session_id.as_deref() == Some("closer-remote-2"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    // page_size=1: the first page is the completed previous turn, the second is running.
    SubagentMuseCloser::sweep(&lifecycle.orch).await.unwrap();
    assert!(lifecycle.vm.delete_attempts().is_empty());
    assert!(
        !lifecycle
            .orch
            .get_session(&lifecycle.caller, &child)
            .await
            .unwrap()
            .archived
    );
    lifecycle.provider.gate.add_permits(1);
    lifecycle.wait_root(&turn).await;
    SubagentMuseCloser::sweep(&lifecycle.orch).await.unwrap();
    let mut deleted = lifecycle.vm.deleted_sessions();
    deleted.sort();
    assert_eq!(deleted, ["closer-remote-1", "closer-remote-2"]);
    assert!(
        lifecycle
            .orch
            .list_tool_operations()
            .await
            .unwrap()
            .iter()
            .all(|s| s.operation.remote_deleted_at.is_some())
    );
    lifecycle.finish().await;
}

#[tokio::test]
async fn muse_task_closer_corrupt_journal_does_not_block_other_child_keys() {
    let lifecycle = Lifecycle::boot().await;
    lifecycle.provider.corrupt_at.store(1, Ordering::SeqCst);
    lifecycle.complete_child().await;
    lifecycle.complete_child().await;
    SubagentMuseCloser::sweep(&lifecycle.orch).await.unwrap();
    assert_eq!(lifecycle.vm.deleted_sessions(), ["closer-remote-2"]);
    let ops = lifecycle.orch.list_tool_operations().await.unwrap();
    let corrupt = ops
        .iter()
        .find(|s| s.operation.remote_session_id.as_deref() == Some("closer-remote-1"))
        .unwrap();
    assert!(corrupt.operation.remote_deleted_at.is_none());
    lifecycle.finish().await;
}

#[tokio::test]
async fn muse_task_closer_startup_tick_closes_restored_completed_child() {
    let lifecycle = Lifecycle::boot().await;
    lifecycle.complete_child().await;
    let child = lifecycle.child_session().await;
    let Lifecycle {
        dir,
        orch,
        model: _,
        vm,
        _pool,
        caller,
        root: _,
        provider,
        config,
    } = lifecycle;
    orch.shutdown().await.unwrap();
    drop(orch);
    let orch = Arc::new(
        Orchestrator::builder(config)
            .register_tool(Arc::new(MuseTaskTool::for_provider(
                provider,
                128 * 1024 * 1024,
                24 * 1024,
            )))
            .build()
            .await
            .unwrap(),
    );
    let ops = orch.list_tool_operations().await.unwrap();
    super::super::pool::restore_job_sessions_from_operations(&ops).await;
    SubagentMuseCloser::spawn(&orch);
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if orch.list_tool_operations().await.unwrap()[0]
                .operation
                .remote_deleted_at
                .is_some()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(vm.deleted_sessions(), ["closer-remote-1"]);
    assert!(orch.get_session(&caller, &child).await.unwrap().archived);
    orch.shutdown().await.unwrap();
    drop(orch);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn muse_task_closer_requires_ack_from_every_account_for_the_same_remote_id() {
    let lifecycle = Lifecycle::boot().await;
    lifecycle.complete_child().await;
    let child = lifecycle.child_session().await;
    let vm2 = FakeVmAccount::new(vec![]);
    let transport2 = FakeMuseVmTransport::new(vec![vm2.clone()]);
    let gateway2 = build_gateway_muse_provider(&[SINGLE_COOKIE], transport2.clone()).await;
    let lease2 = gateway2
        .hub
        .acquire_lease(&GatewayBinding::Pool(GatewayVendor::Muse))
        .unwrap();
    let _pool2 = SharedMusePool::for_gateway_lease(
        &reqwest::Client::new(),
        &lease2.account_key,
        lease2.access_token.expose(),
        Some(transport2),
    );
    *lifecycle.provider.account_override.lock().unwrap() = Some(lease2.account_key.clone());
    lifecycle.provider.same_remote.store(true, Ordering::SeqCst);
    let (_call, turn) = lifecycle.start_child(Some(&child)).await;
    lifecycle.wait_root(&turn).await;
    lifecycle.vm.set_delete_failure_status(Some(503));
    SubagentMuseCloser::sweep(&lifecycle.orch).await.unwrap();
    assert_eq!(vm2.deleted_sessions(), ["closer-remote-1"]);
    assert!(
        lifecycle
            .orch
            .list_tool_operations()
            .await
            .unwrap()
            .iter()
            .all(|s| s.operation.remote_deleted_at.is_none())
    );
    lifecycle.vm.set_delete_failure_status(None);
    SubagentMuseCloser::sweep(&lifecycle.orch).await.unwrap();
    assert_eq!(lifecycle.vm.deleted_sessions(), ["closer-remote-1"]);
    assert!(
        lifecycle
            .orch
            .list_tool_operations()
            .await
            .unwrap()
            .iter()
            .all(|s| s.operation.remote_deleted_at.is_some())
    );
    lifecycle.finish().await;
}

#[tokio::test]
async fn muse_task_closer_preserves_remote_id_shared_with_an_unclosable_key() {
    let lifecycle = Lifecycle::boot().await;
    lifecycle.provider.corrupt_at.store(1, Ordering::SeqCst);
    lifecycle.complete_child().await;
    lifecycle.provider.same_remote.store(true, Ordering::SeqCst);
    lifecycle.complete_child().await;
    SubagentMuseCloser::sweep(&lifecycle.orch).await.unwrap();
    // Marking is global by remote ID in 0.6.0: neither key may be marked/deleted
    // while the other key's closure eligibility cannot be established.
    assert!(lifecycle.vm.delete_attempts().is_empty());
    assert!(
        lifecycle
            .orch
            .list_tool_operations()
            .await
            .unwrap()
            .iter()
            .all(|s| s.operation.remote_deleted_at.is_none())
    );
    lifecycle.finish().await;
}

#[tokio::test]
async fn muse_task_closer_startup_keeps_recovering_child_and_existing_remote_session() {
    let lifecycle = Lifecycle::boot().await;
    lifecycle.provider.block_at.store(1, Ordering::SeqCst);
    let (_call, turn) = lifecycle.start_child(None).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if lifecycle
                .orch
                .list_tool_operations()
                .await
                .unwrap()
                .iter()
                .any(|s| s.operation.remote_session_id.is_some())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let child = lifecycle.child_session().await;
    let Lifecycle {
        dir,
        orch,
        model: _,
        vm,
        _pool,
        caller,
        root,
        provider,
        config,
    } = lifecycle;
    orch.shutdown().await.unwrap();
    drop(orch);
    provider.block_at.store(2, Ordering::SeqCst);
    provider.same_remote.store(true, Ordering::SeqCst);
    let orch = Arc::new(
        Orchestrator::builder(config)
            .register_tool(Arc::new(MuseTaskTool::for_provider(
                provider.clone(),
                128 * 1024 * 1024,
                24 * 1024,
            )))
            .build()
            .await
            .unwrap(),
    );
    let ops = orch.list_tool_operations().await.unwrap();
    super::super::pool::restore_job_sessions_from_operations(&ops).await;
    SubagentMuseCloser::sweep(&orch).await.unwrap();
    assert!(vm.delete_attempts().is_empty());
    assert!(!orch.get_session(&caller, &child).await.unwrap().archived);
    // Shutdown leaves the resumable invocation waiting for the runtime retry policy.
    // Use its public operator resume command rather than waiting through backoff.
    orch.resume_waiting(&caller, &child).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while provider.calls.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        provider
            .restored_requests
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .as_deref(),
        Some("closer-remote-1")
    );
    SubagentMuseCloser::sweep(&orch).await.unwrap();
    assert!(vm.delete_attempts().is_empty());
    assert!(!orch.get_session(&caller, &child).await.unwrap().archived);
    provider.gate.add_permits(1);
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if orch.result(&caller, &root, &turn).await.unwrap().status == RunStatus::Completed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    SubagentMuseCloser::sweep(&orch).await.unwrap();
    assert_eq!(vm.deleted_sessions(), ["closer-remote-1"]);
    orch.shutdown().await.unwrap();
    drop(orch);
    let _ = std::fs::remove_dir_all(dir);
}
