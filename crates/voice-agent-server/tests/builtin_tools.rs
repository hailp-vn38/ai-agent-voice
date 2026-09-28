use voice_agent_server::tools::builtin::{
    EXIT_TOOL_NAME, MAX_GOODBYE_CHARS, exit_tool_definition, parse_exit_args,
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
