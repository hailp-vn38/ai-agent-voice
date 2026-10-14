use super::*;
use crate::providers::llm::{ChatMessage, LlmRequest};
use crate::services::background::models::JobModels;
use tokio_util::sync::CancellationToken;

struct HierarchicalScriptedModels;
#[async_trait::async_trait]
impl JobModels for HierarchicalScriptedModels {
    async fn complete(
        &self,
        _: &str,
        _: &ProviderPin,
        request: LlmRequest,
        _: i64,
        _: CancellationToken,
    ) -> Result<String, JobError> {
        let is_root = request.messages.iter().any(
            |m| matches!(m,ChatMessage::System{content} if content.contains("Root Orchestrator")),
        );
        let first=request.messages.iter().any(|m|matches!(m,ChatMessage::User{content} if content.contains("\"joined_children\":[]")));
        if is_root && first {
            Ok(json!({"action":"delegate","tasks":[
                {"task_key":"research","role":"research","goal":"Find references",
                    "allowed_operations":[]},
                {"task_key":"document","role":"document","goal":"Review documents",
                    "allowed_operations":[]}
            ]})
            .to_string())
        } else if is_root {
            Ok(json!({"action":"finish","result":{
                "summary":"Both independent tasks completed",
                "findings":["Research completed","Document completed"],
                "evidence_refs":[],"unresolved":[]
            }})
            .to_string())
        } else {
            Ok(json!({"action":"finish","result":{
                "status":"completed","summary":"Sub-agent task done",
                "findings":[],"evidence_refs":[],"unresolved":[]
            }})
            .to_string())
        }
    }
}

#[tokio::test]
async fn hierarchical_job_forks_two_children_then_joins_and_finishes() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::execution_harness(
        db.clone(),
        config,
        Arc::new(HierarchicalScriptedModels),
    )
    .await
    .unwrap();
    enable(&runtime.engine).await;
    let receipt = db
        .submit_job(
            &runtime.engine.config,
            request("Research two independent topics"),
            &JobRequester::Admin,
            crate::database::jobs::SubmissionIdentity {
                key: Some("hierarchy-e2e"),
                test_execution_mode: Some("hierarchical_v2"),
                ..Default::default()
            },
            crate::database::unix_seconds().unwrap(),
            || runtime.engine.runtime_ready(),
        )
        .await
        .unwrap();
    let job = wait_for(&runtime.engine, &receipt.job.id, |j| {
        j.status == "succeeded"
    })
    .await;
    assert_eq!(job.execution_mode, "hierarchical_v2");
    assert_eq!(job.agent_runs.len(), 3);
    assert_eq!(
        job.agent_runs
            .iter()
            .filter(|r| r.parent_agent_run_id.is_some())
            .count(),
        2
    );
    assert!(job.agent_runs.iter().all(|r| r.status == "succeeded"));
    assert_eq!(
        job.result.as_ref().unwrap()["summary"],
        "Both independent tasks completed"
    );
    assert_eq!(job.budget_used["llm_calls"], 4);
    let log = db.job_events(&job.id, 0, 100).await.unwrap();
    assert_eq!(
        log.iter()
            .filter(|e| e.event_type == "agent_run_created")
            .count(),
        3
    );
    assert_eq!(
        log.iter()
            .filter(|e| e.event_type == "agent_run_finished")
            .count(),
        3
    );
    assert_eq!(
        log.iter()
            .filter(|e| e.event_type == "join_completed")
            .count(),
        1
    );
    assert_eq!(
        log.iter()
            .filter(|e| e.event_type == "llm_run_started")
            .count(),
        4
    );
    assert!(
        log.iter()
            .any(|e| e.event_type == "llm_run_started" && e.payload["agent_run_id"].is_string())
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn hierarchical_job_rejects_child_authority_expansion() {
    let (db, config, runtime) = harness().await;
    let receipt = db
        .submit_job(
            &config,
            request("Bounded authority"),
            &JobRequester::Admin,
            crate::database::jobs::SubmissionIdentity {
                key: Some("hierarchy-authority"),
                test_execution_mode: Some("hierarchical_v2"),
                ..Default::default()
            },
            crate::database::unix_seconds().unwrap(),
            || runtime.engine.runtime_ready(),
        )
        .await
        .unwrap();
    let id = &receipt.job.id;
    // Foundation harness does not run a model; transition to planning/claimed state.
    let claimed = db.claim_job(id, 2).await.unwrap();
    assert!(claimed);
    let root = db.create_root_run(id).await.unwrap();
    let attempted = db
        .create_children_for_decision(
            id,
            &root.agent_run_id,
            "fake-root-turn",
            &[crate::services::background::model::DelegatedTask {
                task_key: "unapproved".into(),
                role: "research".into(),
                goal: "Attempt a forbidden operation".into(),
                allowed_operations: vec!["uninstalled.execute".into()],
            }],
        )
        .await;
    assert!(matches!(
        attempted,
        Err(JobError::CapabilityUnavailable) | Err(JobError::InvalidPlan)
    ));
    runtime.shutdown().await;
}
