//! Durable Session ownership, transactional batches and release admission.
use super::*;
use crate::database::jobs::V3SessionTask;
use crate::providers::llm::{ChatMessage, ToolCall};
use crate::services::background::model::DelegatedTask;

fn task(key: &str) -> DelegatedTask {
    DelegatedTask {
        task_key: key.into(),
        role: "research".into(),
        goal: "Display title".into(),
        allowed_operations: Vec::new(),
    }
}
async fn root(db: &Database, config: &AppConfig) -> (JobView, AgentRunView, String) {
    let job = db
        .submit_job(
            config,
            request("Owned V3 session"),
            &JobRequester::Admin,
            crate::database::jobs::SubmissionIdentity {
                test_execution_mode: Some("hierarchical_v3"),
                ..Default::default()
            },
            crate::database::unix_seconds().unwrap(),
            || true,
        )
        .await
        .unwrap()
        .job;
    assert!(db.claim_job(&job.id, 2).await.unwrap());
    let run = db.create_root_run(&job.id).await.unwrap();
    let sid = db
        .create_v3_root_session(
            &job.id,
            &run.agent_run_id,
            &job.snapshot.orchestrator,
            "Root system",
            "Root goal",
        )
        .await
        .unwrap();
    (job, run, sid)
}
#[tokio::test]
async fn v3_session_foreign_busy_and_changed_route_are_denied_and_batch_rolls_back() {
    let (db, config, runtime) = harness().await;
    runtime.shutdown().await;
    let (job, run, root_sid) = root(&db, &config).await;
    let children = db
        .create_children_for_decision(&job.id, &run.agent_run_id, "first", &[task("first")])
        .await
        .unwrap();
    let sid = db
        .attach_v3_child_session(
            &job.id,
            &run.agent_run_id,
            &children[0].agent_run_id,
            V3SessionTask {
                agent_key: "research",
                resume_session_id: None,
                pin: &job.snapshot.worker,
                system: "Specialist original",
                prompt: "PRIVATE original prompt",
            },
        )
        .await
        .unwrap();
    assert_eq!(
        db.v3_preflight_child_resume(
            &job.id,
            &run.agent_run_id,
            &sid,
            "research",
            &job.snapshot.worker
        )
        .await
        .unwrap_err(),
        JobError::CapacityUnavailable
    );
    db.start_child_run(&job.id, &children[0].agent_run_id)
        .await
        .unwrap();
    db.v3_append_messages(
        &job.id,
        &sid,
        &[ChatMessage::AssistantText {
            content: "original result".into(),
        }],
    )
    .await
    .unwrap();
    db.finish_child_run(&job.id, &children[0].agent_run_id, "succeeded", None, None)
        .await
        .unwrap();
    db.join_children(&job.id, &run.agent_run_id, "first")
        .await
        .unwrap();
    let mut changed = job.snapshot.worker.clone();
    changed.digest = "changed".into();
    assert_eq!(
        db.v3_preflight_child_resume(&job.id, &run.agent_run_id, &sid, "research", &changed)
            .await
            .unwrap_err(),
        JobError::InvalidArguments
    );
    let (other, other_run, _) = root(&db, &config).await;
    assert_eq!(
        db.v3_preflight_child_resume(
            &other.id,
            &other_run.agent_run_id,
            &sid,
            "research",
            &job.snapshot.worker
        )
        .await
        .unwrap_err(),
        JobError::InvalidArguments
    );
    assert!(db.v3_read_messages(&other.id, &sid).await.is_err());
    let calls = vec![
        ToolCall {
            id: "one".into(),
            name: "subagent".into(),
            arguments: json!({}),
        },
        ToolCall {
            id: "two".into(),
            name: "subagent".into(),
            arguments: json!({}),
        },
    ];
    let before = db.job_events(&job.id, 0, 100).await.unwrap().len();
    let error = db
        .create_v3_children(
            &job.id,
            &run.agent_run_id,
            "atomic",
            &[task("one"), task("two")],
            &[
                (None, "system", "first prompt"),
                (Some("foreign"), "system", "second prompt"),
            ]
            .iter()
            .map(|(resume, system, prompt)| V3SessionTask {
                agent_key: "research",
                resume_session_id: *resume,
                pin: &job.snapshot.worker,
                system,
                prompt,
            })
            .collect::<Vec<_>>(),
            &calls,
        )
        .await
        .unwrap_err();
    assert_eq!(error, JobError::InvalidArguments);
    assert_eq!(db.job_view(&job.id).await.unwrap().agent_runs.len(), 2);
    assert_eq!(db.job_events(&job.id, 0, 100).await.unwrap().len(), before);
    assert_eq!(
        db.v3_read_messages(&job.id, &root_sid).await.unwrap().len(),
        2
    );
    assert!(
        sqlx::query("UPDATE job_agent_sessions SET provider_route_json='{}' WHERE session_id=?")
            .bind(&sid)
            .execute(db.pool())
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE job_agent_messages SET payload_json='{}' WHERE session_id=?")
            .bind(&sid)
            .execute(db.pool())
            .await
            .is_err()
    );
    // Successful resume retains systems/history and claims the same Session.
    let calls = &calls[..1];
    // Evidence produced by a previous Run belongs to this Session on resume.
    sqlx::query("INSERT INTO job_steps(job_id,step_key,plan_revision,agent_run_id,status,created_at,updated_at) VALUES(?,'prior-evidence',1,?,'succeeded',0,0)")
        .bind(&job.id).bind(&children[0].agent_run_id).execute(db.pool()).await.unwrap();
    sqlx::query("INSERT INTO job_artifacts(id,job_id,step_key,relative_path,media_type,size_bytes,sha256,created_at) VALUES('prior-artifact',?,'prior-evidence','prior/path','text/plain',1,'digest',0)")
        .bind(&job.id).execute(db.pool()).await.unwrap();
    let resumed = db
        .create_v3_children(
            &job.id,
            &run.agent_run_id,
            "resume",
            &[task("resume")],
            &[(
                Some(sid.as_str()),
                "must not replace system",
                "new user task",
            )]
            .iter()
            .map(|(resume, system, prompt)| V3SessionTask {
                agent_key: "research",
                resume_session_id: *resume,
                pin: &job.snapshot.worker,
                system,
                prompt,
            })
            .collect::<Vec<_>>(),
            calls,
        )
        .await
        .unwrap();
    assert_eq!(
        db.v3_session_for_run(&job.id, &resumed[0].agent_run_id)
            .await
            .unwrap(),
        sid
    );
    assert_eq!(
        db.v3_child_artifact_refs(&job.id, &resumed[0].agent_run_id)
            .await
            .unwrap(),
        vec!["prior-artifact"]
    );
    assert!(
        db.v3_child_artifact_refs(&other.id, &other_run.agent_run_id)
            .await
            .unwrap()
            .is_empty()
    );
    let messages = db.v3_read_messages(&job.id, &sid).await.unwrap();
    assert!(matches!(&messages[0],ChatMessage::System{content} if content=="Specialist original"));
    assert!(
        matches!(messages.last().unwrap(),ChatMessage::User{content} if content=="new user task")
    );
    assert_eq!(
        db.v3_preflight_child_resume(
            &job.id,
            &run.agent_run_id,
            &sid,
            "research",
            &job.snapshot.worker
        )
        .await
        .unwrap_err(),
        JobError::CapacityUnavailable
    );
}
#[tokio::test]
async fn v3_unknown_call_intent_cannot_resume_or_be_replayed_after_restart() {
    let (db, config, runtime) = harness().await;
    runtime.shutdown().await;
    let (job, run, _) = root(&db, &config).await;
    let child = db
        .create_children_for_decision(&job.id, &run.agent_run_id, "first", &[task("first")])
        .await
        .unwrap()
        .remove(0);
    let sid = db
        .attach_v3_child_session(
            &job.id,
            &run.agent_run_id,
            &child.agent_run_id,
            V3SessionTask {
                agent_key: "research",
                resume_session_id: None,
                pin: &job.snapshot.worker,
                system: "specialist",
                prompt: "task",
            },
        )
        .await
        .unwrap();
    db.start_child_run(&job.id, &child.agent_run_id)
        .await
        .unwrap();
    db.v3_append_messages(
        &job.id,
        &sid,
        &[ChatMessage::AssistantToolCalls {
            calls: vec![ToolCall {
                id: "external".into(),
                name: "plugin".into(),
                arguments: json!({}),
            }],
        }],
    )
    .await
    .unwrap();
    assert!(
        db.v3_append_messages(
            &job.id,
            &sid,
            &[ChatMessage::User {
                content: "try again".into()
            }]
        )
        .await
        .is_err()
    );
    sqlx::query("INSERT INTO job_steps(job_id,step_key,plan_revision,agent_run_id,status,attempt,attempt_id,attempt_state,worker_instance_id,created_at,updated_at) VALUES(?,'external',1,?,'running',1,'attempt','start_requested','worker',0,0)")
        .bind(&job.id).bind(&child.agent_run_id).execute(db.pool()).await.unwrap();
    let reopened = Database::connect(&config.database).await.unwrap();
    reopened
        .recover_jobs(crate::database::unix_seconds().unwrap())
        .await
        .unwrap();
    let recovered = reopened.job_view(&job.id).await.unwrap();
    assert_eq!(recovered.status, "failed");
    assert_eq!(
        recovered.steps[0].attempt_state.as_deref(),
        Some("unknown_outcome")
    );
    assert_eq!(
        reopened
            .reserve_plugin_attempt(&job.id, "external", "replay")
            .await
            .unwrap_err(),
        JobError::Terminal
    );
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM job_agent_messages WHERE session_id=? AND kind='assistant_tool_calls'").bind(&sid).fetch_one(reopened.pool()).await.unwrap();
    assert_eq!(count, 1, "unknown intent remains as evidence");
}
#[tokio::test]
async fn v3_release_attestation_must_match_both_exact_pins() {
    let (db, config, runtime) = harness().await;
    runtime.shutdown().await;
    let pin = db
        .background_provider_pin(&config, "config:llm:planner")
        .await
        .unwrap();
    let mut config = (*config).clone();
    let raw = serde_json::to_string(&pin).unwrap();
    config.background.hierarchical_v3_qualified_pins = Some([raw.clone(), raw]);
    let receipt = db
        .submit_job(
            &config,
            request("Qualified pair"),
            &JobRequester::Admin,
            crate::database::jobs::SubmissionIdentity {
                test_execution_mode: Some("hierarchical_v3"),
                ..Default::default()
            },
            crate::database::unix_seconds().unwrap(),
            || true,
        )
        .await
        .unwrap();
    assert_eq!(receipt.job.execution_mode, "hierarchical_v3");
    let mut different = pin;
    different.revision += 1;
    config
        .background
        .hierarchical_v3_qualified_pins
        .as_mut()
        .unwrap()[1] = serde_json::to_string(&different).unwrap();
    assert_eq!(
        db.submit_job(
            &config,
            request("Changed pair"),
            &JobRequester::Admin,
            crate::database::jobs::SubmissionIdentity {
                test_execution_mode: Some("hierarchical_v3"),
                ..Default::default()
            },
            crate::database::unix_seconds().unwrap(),
            || true
        )
        .await
        .unwrap_err(),
        JobError::ConfigurationChanged
    );
}

#[tokio::test]
async fn v3_concurrent_resume_claims_one_runner_and_appends_only_one_prompt() {
    let (db, config, runtime) = harness().await;
    runtime.shutdown().await;
    let (job, run, _) = root(&db, &config).await;
    let first = db
        .create_children_for_decision(&job.id, &run.agent_run_id, "original", &[task("original")])
        .await
        .unwrap()
        .remove(0);
    let sid = db
        .attach_v3_child_session(
            &job.id,
            &run.agent_run_id,
            &first.agent_run_id,
            V3SessionTask {
                agent_key: "research",
                resume_session_id: None,
                pin: &job.snapshot.worker,
                system: "original system",
                prompt: "original prompt",
            },
        )
        .await
        .unwrap();
    db.start_child_run(&job.id, &first.agent_run_id)
        .await
        .unwrap();
    db.finish_child_run(&job.id, &first.agent_run_id, "succeeded", None, None)
        .await
        .unwrap();
    db.join_children(&job.id, &run.agent_run_id, "original")
        .await
        .unwrap();
    let attempts = db
        .create_children_for_decision(
            &job.id,
            &run.agent_run_id,
            "racing",
            &[task("a"), task("b")],
        )
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        db.attach_v3_child_session(
            &job.id,
            &run.agent_run_id,
            &attempts[0].agent_run_id,
            V3SessionTask {
                agent_key: "research",
                resume_session_id: Some(&sid),
                pin: &job.snapshot.worker,
                system: "unused",
                prompt: "resume a"
            }
        ),
        db.attach_v3_child_session(
            &job.id,
            &run.agent_run_id,
            &attempts[1].agent_run_id,
            V3SessionTask {
                agent_key: "research",
                resume_session_id: Some(&sid),
                pin: &job.snapshot.worker,
                system: "unused",
                prompt: "resume b"
            }
        )
    );
    assert_eq!([&a, &b].iter().filter(|r| r.is_ok()).count(), 1);
    let denied = match a {
        Err(error) => error,
        Ok(_) => b.unwrap_err(),
    };
    assert_eq!(denied, JobError::CapacityUnavailable);
    let messages = db.v3_read_messages(&job.id, &sid).await.unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|m| matches!(m,ChatMessage::User{content} if content.starts_with("resume ")))
            .count(),
        1
    );
    let occupied: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_agent_runs WHERE session_id=? AND status IN ('queued','running')",
    )
    .bind(&sid)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(occupied, 1);
}
