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
