use voice_agent_server::tools::builtin::{
    EXIT_TOOL_NAME, MAX_GOODBYE_CHARS, MAX_TEMPLATE_KEY_CHARS, SWITCH_TEMPLATE_TOOL_NAME,
    exit_tool_definition, parse_exit_args, parse_switch_template_args,
    switch_template_tool_definition,
};

#[test]
fn exit_definition_requires_a_single_goodbye_without_extra_properties() {
    let definition = exit_tool_definition();

    assert_eq!(definition.name, EXIT_TOOL_NAME);
    assert_eq!(
        definition.parameters["required"],
        serde_json::json!(["say_goodbye"])
    );
    assert_eq!(definition.parameters["additionalProperties"], false);
}

#[test]
fn exit_arguments_trim_valid_goodbye_and_reject_blank_or_oversized_values() {
    let parsed = parse_exit_args(&serde_json::json!({"say_goodbye": "  Tạm biệt!  "}))
        .expect("a non-empty goodbye is valid");
    assert_eq!(parsed.say_goodbye, "Tạm biệt!");

    assert!(parse_exit_args(&serde_json::json!({"say_goodbye": "   "})).is_err());
    assert!(
        parse_exit_args(&serde_json::json!({"say_goodbye": "Tạm biệt", "extra": true})).is_err()
    );
    assert!(
        parse_exit_args(&serde_json::json!({
            "say_goodbye": "x".repeat(MAX_GOODBYE_CHARS + 1)
        }))
        .is_err()
    );
}

#[test]
fn switch_definition_names_exactly_the_templates_this_session_admitted() {
    let definition = switch_template_tool_definition(&["primary", "sales"]);

    assert_eq!(definition.name, SWITCH_TEMPLATE_TOOL_NAME);
    assert_eq!(
        definition.parameters["required"],
        serde_json::json!(["template"])
    );
    assert_eq!(definition.parameters["additionalProperties"], false);
    assert_eq!(
        definition.parameters["properties"]["template"]["enum"],
        serde_json::json!(["primary", "sales"]),
        "the model must only ever be offered this session's own candidates"
    );
    assert!(
        SWITCH_TEMPLATE_TOOL_NAME.contains('.'),
        "the dotted name namespaces a server action the model calls verbatim"
    );
    assert!(
        !EXIT_TOOL_NAME.contains('.'),
        "a dotted name is impossible for a sanitized Device MCP tool, so builtins cannot collide"
    );
}

#[test]
fn switch_arguments_trim_a_template_key_and_reject_anything_else() {
    let parsed = parse_switch_template_args(&serde_json::json!({"template": "  sales  "}))
        .expect("a known-shaped key is structurally valid");
    assert_eq!(parsed.template, "sales");

    assert!(parse_switch_template_args(&serde_json::json!({"template": "   "})).is_err());
    assert!(parse_switch_template_args(&serde_json::json!({})).is_err());
    assert!(
        parse_switch_template_args(&serde_json::json!({"template": "sales", "extra": true}))
            .is_err()
    );
    assert!(
        parse_switch_template_args(&serde_json::json!({
            "template": "x".repeat(MAX_TEMPLATE_KEY_CHARS + 1)
        }))
        .is_err(),
        "an unbounded key must never reach the catalog lookup"
    );
}
