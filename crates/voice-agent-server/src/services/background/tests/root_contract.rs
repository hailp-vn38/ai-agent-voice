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
