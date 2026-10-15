use super::{
    results::{child_evidence_allowed, plugin_tool_result},
    tools::{root_call_batch_valid, root_tools},
};
use serde_json::json;

#[test]
fn plugin_tool_result_exposes_only_persisted_artifact_ids() {
    // Reproduction: fetch_url succeeded but Child did not receive artifact IDs.
    // A Child must not invent evidence_refs when invoking submit_result.
    let refs = vec!["23876c4f-9bc2-4fcb-8d88-a5c4756552ba".to_owned()];
    let payload = plugin_tool_result(
        "t_fixture",
        "succeeded",
        None,
        Some(r#"{"title":"VFF","text":"Article content"}"#),
        &refs,
    );
    assert_eq!(payload["step_key"], "t_fixture");
    assert_eq!(payload["evidence_refs"], json!(refs));
    assert_eq!(payload["status"], "succeeded");
    assert!(payload["result_excerpt"].is_string());
    assert!(payload.get("arguments").is_none());
}

#[test]
fn evidence_refs_cannot_cross_child_or_invent_artifact_ids() {
    let child_refs = vec!["artifact_owned_by_this_child".to_owned()];
    assert!(child_evidence_allowed(
        &["artifact_owned_by_this_child".to_owned()],
        &child_refs,
    ));
    assert!(!child_evidence_allowed(
        &["artifact_owned_by_other_child".to_owned()],
        &child_refs,
    ));
    assert!(!child_evidence_allowed(
        &["https://www.24h.com.vn/article".to_owned()],
        &child_refs,
    ));
}

#[test]
fn subagent_schema_exposes_full_prompt_and_session_resume() {
    let tools = root_tools();
    assert!(tools.iter().any(|tool| tool.name == "subagent"));
    assert!(tools.iter().any(|tool| tool.name == "finish_job"));
    let schema = &tools
        .iter()
        .find(|tool| tool.name == "subagent")
        .unwrap()
        .parameters;
    assert!(schema["properties"].get("prompt").is_some());
    assert!(schema["properties"].get("sessionID").is_some());
}

#[test]
fn invalid_root_batches_cannot_execute_any_children() {
    use crate::providers::llm::ToolCall;
    let sub = ToolCall {
        id: "call_1".into(),
        name: "subagent".into(),
        arguments: json!({"agent":"research","description":"Investigate","prompt":"Full child task",
            "model":null,"sessionID":null,"background":false}),
    };
    let finish = ToolCall {
        id: "call_2".into(),
        name: "finish_job".into(),
        arguments: json!({"summary":"Done","findings":[],"unresolved":[]}),
    };
    let seen = std::collections::HashSet::new();
    assert!(root_call_batch_valid(std::slice::from_ref(&sub), &seen));
    assert!(!root_call_batch_valid(&[sub.clone(), finish], &seen));
    assert!(!root_call_batch_valid(&[sub.clone(), sub.clone()], &seen));
}

#[test]
fn root_source_schema_and_ids_reject_every_malformed_batch() {
    use crate::providers::llm::ToolCall;
    let valid = ToolCall {
        id: "call".into(),
        name: "subagent".into(),
        arguments: json!({
        "agent":"general","description":"Title","prompt":"Full task", "model":null,
        "sessionID":null,"background":false}),
    };
    let seen = std::collections::HashSet::new();
    for field in [
        "agent",
        "description",
        "prompt",
        "model",
        "sessionID",
        "background",
    ] {
        let mut bad = valid.clone();
        bad.arguments.as_object_mut().unwrap().remove(field);
        assert!(!root_call_batch_valid(&[bad], &seen), "missing {field}");
    }
    for id in ["", "duplicate"] {
        let mut bad = valid.clone();
        bad.id = id.into();
        let batch = if id.is_empty() {
            vec![bad]
        } else {
            vec![bad.clone(), bad]
        };
        assert!(!root_call_batch_valid(&batch, &seen));
    }
    let mut unknown = valid.clone();
    unknown.name = "shell".into();
    assert!(!root_call_batch_valid(&[unknown], &seen));
    let mut extra = valid.clone();
    extra.arguments["authority"] = json!("admin");
    assert!(!root_call_batch_valid(&[extra], &seen));
    let mut other = valid.clone();
    other.id = "two".into();
    let mut third = valid.clone();
    third.id = "three".into();
    assert!(!root_call_batch_valid(
        &[valid.clone(), other, third],
        &seen
    ));
    assert!(!root_call_batch_valid(
        &[valid],
        &std::collections::HashSet::from(["call".into()])
    ));
}

fn news_summary_fixture(truncated: bool) -> crate::services::background::JobView {
    use crate::services::background::model::{
        AgentRunView, JobRequest, JobSnapshot, JobStepView, ProviderPin,
    };
    let url = "https://news.example/asian-cup.html";
    let child_result = json!({
        "status":"completed",
        "summary":"AFC has NOT made an official decision to postpone the tournament.",
        "findings":[],
        "evidence_refs":[],
        "unresolved":[]
    });
    let child = AgentRunView {
        agent_run_id: "child".into(),
        parent_agent_run_id: Some("root".into()),
        fork_group_id: Some("fork".into()),
        member_ordinal: Some(0),
        role: "research".into(),
        goal_excerpt: "Summarize article".into(),
        allowed_operations: vec!["core.web.extract".into()],
        status: "succeeded".into(),
        result: Some(child_result),
        error_code: None,
        started_at: None,
        finished_at: None,
    };
    let step = JobStepView {
        step_key: "step_extract".into(),
        agent_run_id: Some("child".into()),
        agent_turn_id: Some("turn".into()),
        status: "succeeded".into(),
        plugin_id: Some("core".into()),
        operation: Some("web.extract".into()),
        error_code: None,
        attempt_state: Some("finished".into()),
        instruction: "web.extract".into(),
        required: true,
        dependencies: vec![],
        input_refs: vec![],
        arguments: json!({"url":url,"mode":"article","max_chars":12000}),
        output_schema: json!({}),
        result: Some(json!({
            "url":url,"text":"AFC has NOT made an official decision to postpone the tournament.",
            "status":200,"content_type":"text/html","title":"Asian Cup",
            "truncated":truncated,"next_offset":if truncated {Some(62)}else{None},
            "body_limit_reached":false,
            "provenance":[{"source":"web","urls":[url],"capabilities":["web_extract"]}]
        })),
    };
    let pin = ProviderPin {
        reference: "config:llm:planner".into(),
        incarnation: 1,
        revision: 1,
        digest: "fixture".into(),
    };
    crate::services::background::JobView {
        id: "fixture-job".into(),
        request: JobRequest {
            goal: format!("tóm tắt nội dung bài viết tại URL: {url}"),
            output_format: "text".into(),
            allow_partial: false,
            required_sources: vec![],
            required_capabilities: vec![],
        },
        status: "running".into(),
        execution_mode: "hierarchical_v3".into(),
        agent_runs: vec![child],
        requester_scope: "admin:deployment".into(),
        snapshot: JobSnapshot {
            mode: None,
            orchestrator: pin.clone(),
            worker: pin,
            limits: crate::services::background::tests::config()
                .background
                .limits,
        },
        plugin_choices: json!([{
            "plugin_id":"core","operation":"web.extract",
            "sources":["web"],"provides":["web_extract"]
        }]),
        settings_revision: 1,
        deadline_at: 0,
        created_at: "2026-10-14T00:00:00Z".into(),
        error_code: None,
        scheduler_seen_at: None,
        plan_revision: 0,
        retry_of_job_id: None,
        result: None,
        budget_used: json!({}),
        steps: vec![step],
        artifacts: vec![],
    }
}

#[test]
fn url_summary_with_truncated_web_steps_is_not_verified_success() {
    let job = news_summary_fixture(true);
    let (status, coverage) = super::results::verified_coverage(&job).unwrap();
    assert_eq!(status, "failed");
    assert_eq!(coverage["coverage_basis"], "goal_url_unverified");
    assert_eq!(coverage["source_complete"], false);
    assert!(
        coverage["verified_source_urls"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(super::results::grounded_single_child_summary(&job, &coverage).is_none());
}

#[test]
fn verified_article_with_unresolved_factual_question_still_satisfies_requirements() {
    // The story says the organizing body has not decided whether the event
    // will be postponed. That is a factual uncertainty, not unfinished work.
    let mut job = news_summary_fixture(false);
    job.agent_runs[0].result.as_mut().unwrap()["unresolved"] =
        json!(["AFC chưa công bố quyết định chính thức về lịch tổ chức."]);
    let (status, coverage) = super::results::verified_coverage(&job).unwrap();
    assert_eq!(status, "succeeded");
    assert_eq!(coverage["coverage_basis"], "goal_url_verified");
    assert_eq!(coverage["source_complete"], true);
    assert!(coverage["missing_sources"].as_array().unwrap().is_empty());
    assert!(
        coverage["missing_capabilities"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(job.artifacts.is_empty());
}

#[test]
fn unresolved_information_cannot_hide_incomplete_requested_url() {
    let mut job = news_summary_fixture(true);
    job.agent_runs[0].result.as_mut().unwrap()["unresolved"] =
        json!(["AFC chưa công bố quyết định chính thức."]);
    let (status, coverage) = super::results::verified_coverage(&job).unwrap();
    assert_eq!(status, "failed");
    assert_eq!(coverage["coverage_basis"], "goal_url_unverified");
    assert_eq!(coverage["source_complete"], false);
}

#[test]
fn verified_article_preserves_child_qualification_and_reports_source() {
    let job = news_summary_fixture(false);
    let (status, coverage) = super::results::verified_coverage(&job).unwrap();
    assert_eq!(status, "succeeded");
    assert_eq!(coverage["coverage_basis"], "goal_url_verified");
    assert_eq!(
        coverage["verified_source_urls"][0],
        "https://news.example/asian-cup.html"
    );
    assert_eq!(coverage["source_complete"], true);
    let summary = super::results::grounded_single_child_summary(&job, &coverage).unwrap();
    assert!(summary.contains("NOT made an official decision"));
    let child = super::results::ChildOutcome::from_run(
        "child-session",
        &job.agent_runs[0],
        vec!["https://news.example/asian-cup.html".into()],
    )
    .unwrap();
    let outcome = serde_json::to_value(child).unwrap();
    assert_eq!(
        outcome["verified_source_urls"][0],
        "https://news.example/asian-cup.html"
    );
    assert!(
        outcome["artifact_refs"].as_array().unwrap().is_empty(),
        "URLs must not be masqueraded as artifact IDs"
    );
}

#[test]
fn web_observation_is_plain_article_text_with_pagination_instead_of_broken_json() {
    let text = "AFC has not made any official decision.";
    let step_result = json!({
        "url":"https://news.example/article","title":"Asian Cup",
        "status":200,"text":text,"truncated":false,
        "next_offset":null,"body_limit_reached":false,
        "provenance":[{"source":"web","urls":["https://news.example/article"],
                       "capabilities":["web_extract"]}]
    })
    .to_string();
    let observation =
        plugin_tool_result("step_article", "succeeded", None, Some(&step_result), &[]);
    assert_eq!(observation["result_excerpt"], text);
    assert_eq!(observation["source_url"], "https://news.example/article");
    assert_eq!(observation["next_offset"], serde_json::Value::Null);
    assert_eq!(observation["observation_truncated"], false);
    assert_eq!(observation["truncated"], false);
}

#[test]
fn web_observation_must_report_when_returned_text_is_too_large_for_child_context() {
    let text = "X".repeat(6200);
    let step_result = json!({
        "url":"https://news.example/article","title":"Asian Cup",
        "status":200,"text":text,"truncated":false,
        "next_offset":null,"body_limit_reached":false
    })
    .to_string();
    let observation = plugin_tool_result("step_large", "succeeded", None, Some(&step_result), &[]);
    assert_eq!(
        observation["result_excerpt"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        6000
    );
    assert_eq!(observation["observation_truncated"], true);
}

#[test]
fn verified_sources_are_scoped_to_the_owned_child_runs() {
    let job = news_summary_fixture(false);
    let foreign = vec!["unrelated_child".to_owned()];
    let owned = vec!["child".to_owned()];
    assert!(
        crate::services::background::coverage::completed_web_sources(&job, Some(&foreign))
            .is_empty()
    );
    assert_eq!(
        crate::services::background::coverage::completed_web_sources(&job, Some(&owned)),
        vec!["https://news.example/asian-cup.html".to_owned()]
    );
}

#[test]
fn finish_job_schema_excludes_llm_authored_evidence_refs() {
    use crate::providers::llm::ToolCall;
    let tools = root_tools();
    let definition = tools.iter().find(|t| t.name == "finish_job").unwrap();
    assert!(
        definition.parameters["properties"]
            .get("evidence_refs")
            .is_none()
    );
    let valid = ToolCall {
        id: "finish_clean".into(),
        name: "finish_job".into(),
        arguments: json!({"summary":"AFC has not decided to postpone the cup.",
            "findings":[], "unresolved":[]}),
    };
    assert!(root_call_batch_valid(
        std::slice::from_ref(&valid),
        &Default::default()
    ));
    let mut legacy = valid;
    legacy.arguments["evidence_refs"] = json!(["https://news.example/asian-cup.html"]);
    assert!(!root_call_batch_valid(&[legacy], &Default::default()));
}

#[test]
fn finish_job_artifact_references_are_resolved_by_rust_only() {
    let mut job = news_summary_fixture(false);
    assert_eq!(
        super::results::server_evidence_refs(&job),
        Vec::<String>::new()
    );
    // Verified source URLs are NOT artifacts.
    assert_eq!(
        super::results::verified_coverage(&job).unwrap().1["source_complete"],
        true
    );
    job.artifacts
        .push(crate::services::background::model::JobArtifactView {
            id: "owned-artifact".into(),
            step_key: None,
            filename: "owned-artifact".into(),
            created_at: "1970-01-01T00:00:00Z".into(),
            media_type: "text/plain".into(),
            size_bytes: 5,
            sha256: "0123456789".into(),
        });
    assert_eq!(
        super::results::server_evidence_refs(&job),
        vec!["owned-artifact"]
    );
}
