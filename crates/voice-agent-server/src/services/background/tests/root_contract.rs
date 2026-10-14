use super::*;

fn delegate(operations: Value) -> Value {
    json!({"decision":{"action":"delegate","tasks":[{
        "task_key":"research","role":"research","goal":"Read references",
        "allowed_operations":operations
    }]}})
}

#[test]
fn root_schema_limits_operations_to_frozen_names() {
    let schema = root_response_schema(&["web-extractor.fetch_url".into(), "documents.read".into()]);
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(validator.is_valid(&delegate(json!([
        "web-extractor.fetch_url",
        "documents.read"
    ]))));
    assert!(validator.is_valid(&delegate(json!([]))));
    for operation in ["web-extractor.search", "web.search", "fetch_url"] {
        assert!(!validator.is_valid(&delegate(json!([operation]))));
    }
}

#[test]
fn root_schema_with_empty_catalog_only_allows_empty_operations() {
    let schema = root_response_schema(&[]);
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(validator.is_valid(&delegate(json!([]))));
    assert!(!validator.is_valid(&delegate(json!(["web-extractor.fetch_url"]))));
    assert!(
        validator.is_valid(&json!({"decision":{"action":"finish","result":{
            "summary":"No tools available","findings":[],"evidence_refs":[],"unresolved":[]
        }}}))
    );
}

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
fn v3_subagent_schema_exposes_full_prompt_and_session_resume() {
    let tools = v3_root_tools();
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
fn v3_invalid_root_batches_cannot_execute_any_children() {
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
        arguments: json!({"summary":"Done","findings":[],"evidence_refs":[],"unresolved":[]}),
    };
    let seen = std::collections::HashSet::new();
    assert!(v3_root_call_batch_valid(std::slice::from_ref(&sub), &seen));
    assert!(!v3_root_call_batch_valid(&[sub.clone(), finish], &seen));
    assert!(!v3_root_call_batch_valid(
        &[sub.clone(), sub.clone()],
        &seen
    ));
}

#[test]
fn v3_root_source_schema_and_ids_reject_every_malformed_batch() {
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
        assert!(!v3_root_call_batch_valid(&[bad], &seen), "missing {field}");
    }
    for id in ["", "duplicate"] {
        let mut bad = valid.clone();
        bad.id = id.into();
        let batch = if id.is_empty() {
            vec![bad]
        } else {
            vec![bad.clone(), bad]
        };
        assert!(!v3_root_call_batch_valid(&batch, &seen));
    }
    let mut unknown = valid.clone();
    unknown.name = "shell".into();
    assert!(!v3_root_call_batch_valid(&[unknown], &seen));
    let mut extra = valid.clone();
    extra.arguments["authority"] = json!("admin");
    assert!(!v3_root_call_batch_valid(&[extra], &seen));
    let mut other = valid.clone();
    other.id = "two".into();
    let mut third = valid.clone();
    third.id = "three".into();
    assert!(!v3_root_call_batch_valid(
        &[valid.clone(), other, third],
        &seen
    ));
    assert!(!v3_root_call_batch_valid(
        &[valid],
        &std::collections::HashSet::from(["call".into()])
    ));
}
