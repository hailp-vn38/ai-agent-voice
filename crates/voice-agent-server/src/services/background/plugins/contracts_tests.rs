use super::*;
#[test]
fn projection_keeps_optional_nullable_but_source_schema_decides() {
    let source = serde_json::json!({"type":"object","properties":{
            "url":{"type":"string","minLength":10},
            "max_chars":{"type":"integer","minimum":1}
        },"required":["url"],"additionalProperties":false});
    let projected = strict_projection(&source).unwrap();
    assert_eq!(
        projected["required"],
        serde_json::json!(["max_chars", "url"])
    );
    assert_eq!(
        projected["properties"]["max_chars"]["type"],
        serde_json::json!(["integer", "null"])
    );
    let mut args = serde_json::json!({"url":"https://example.com","max_chars":null});
    normalize_tool_arguments(&source, &mut args);
    assert_eq!(args, serde_json::json!({"url":"https://example.com"}));
    assert!(jsonschema::validator_for(&source).unwrap().is_valid(&args));
}
#[test]
fn unsupported_schema_cannot_be_projected() {
    assert!(strict_projection(&serde_json::json!({"$ref":"https://example.com"})).is_none());
}

#[test]
fn frozen_document_analysis_tool_keeps_source_oneof_not_authoritative() {
    let schema: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/document-analysis/schemas/analyze_document.input.json"
    )))
    .unwrap();
    let choice = FrozenChoice {
        plugin_id: "document-analysis".into(),
        operation: "analyze_document".into(),
        version: "1.0.0".into(),
        package_digest: "example-digest".into(),
        installation_digest: "example-installation".into(),
    };
    let (tools, bindings) = compile_child_tools(&[(
        choice,
        serde_json::json!({
            "description": "Analyze a document",
            "input_schema": schema,
            "output_schema": {"type":"object"}
        }),
    )])
    .unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "p_document_analysis_analyze_document");
    assert!(bindings.contains_key(&tools[0].name));
    let projected = jsonschema::validator_for(&tools[0].parameters).unwrap();
    let (_, contract) = &bindings[&tools[0].name];
    let original = jsonschema::validator_for(&contract["input_schema"]).unwrap();
    let mut valid = serde_json::json!({
        "format":"markdown","content_base64":null,"text":"Hello",
        "filename":null,"max_chars":null
    });
    assert!(projected.is_valid(&valid));
    normalize_tool_arguments(&contract["input_schema"], &mut valid);
    assert!(original.is_valid(&valid));
    let mut invalid = serde_json::json!({
        "format":"pdf","content_base64":null,"text":"Hello",
        "filename":null,"max_chars":null
    });
    assert!(projected.is_valid(&invalid));
    normalize_tool_arguments(&contract["input_schema"], &mut invalid);
    assert!(
        !original.is_valid(&invalid),
        "Rust must reject PDF without base64 and with text"
    );
}


#[test]
fn legacy_external_and_core_frozen_choices_share_one_tool_compiler() {
    let external = serde_json::json!({
        "plugin_id":"fixture-research","version":"1.0.0",
        "package_digest":"package","installation_digest":"installation",
        "operation":"collect_topic"
    });
    let parsed = super::super::decode_choice(&external).unwrap();
    assert_eq!(parsed.plugin_id, "fixture-research");
    assert!(!crate::tools::core::is_core(&parsed));
    assert!(crate::tools::core::matches_frozen(&external, &parsed));

    let core_raw = crate::tools::core::frozen_choices();
    let choice = super::super::decode_choice(&core_raw[0]).unwrap();
    let contract = crate::tools::core::contract(&choice).unwrap();
    let (defs, bindings) = compile_child_tools(&[(choice, contract)]).unwrap();
    assert_eq!(defs[0].name, "p_core_web_fetch");
    assert!(bindings.contains_key("p_core_web_fetch"));
    assert_eq!(
        defs[0].parameters["properties"]["max_chars"]["type"],
        serde_json::json!(["integer", "null"])
    );

    let mut invalid = core_raw[0].clone();
    invalid["contract_revision"] = serde_json::json!("changed");
    assert!(super::super::decode_choice(&invalid).is_err());
}
