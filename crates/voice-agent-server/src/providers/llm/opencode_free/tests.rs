use super::*;

fn config(model: &str) -> OpenCodeFreeConfig {
    OpenCodeFreeConfig {
        model: model.into(),
        timeout_ms: 60_000,
        supports_responses: false,
        supports_structured_outputs: false,
        supports_function_calling: false,
    }
}

#[test]
fn accepts_free_chat_ids_and_rejects_paid_and_unsafe_ids() {
    assert!(config("nemotron-3-ultra-free").valid());
    assert!(config("future-2027-free").valid());
    assert!(config("space-bunny-free").valid());
    assert!(!config("space-bunny").valid());
    assert!(!config("claude-opus-4-5").valid());
    assert!(!config("a/../../b-free").valid());
}

#[test]
fn admin_config_and_deployment_discriminator_are_consistent() {
    let canonical = crate::database::provider_config::validate(
        "opencode_free",
        &serde_json::json!({
            "model": "space-bunny-free",
            "timeout_ms": 60000,
        }),
    )
    .unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&canonical).unwrap();
    // The runtime loader strips this Admin-only bound before typed decoding.
    value.as_object_mut().unwrap().remove("max_tokens");
    value["adapter"] = serde_json::json!("opencode_free");
    let instance: crate::config::LlmInstanceConfig = serde_json::from_value(value).unwrap();
    assert_eq!(instance.adapter(), "opencode_free");

    assert!(
        crate::database::provider_config::validate(
            "opencode_free",
            &serde_json::json!({
                "model": "claude-sonnet-5",
                "timeout_ms": 60000,
            }),
        )
        .is_err()
    );
    assert!(
        crate::database::provider_config::validate(
            "opencode_free",
            &serde_json::json!({
                "model": "nemotron-3-ultra-free",
                "base_url": "https://example.com",
            }),
        )
        .is_err()
    );
}

#[test]
fn builds_without_network_or_user_credentials() {
    let provider = OpenCodeFreeProvider::build(&config("nemotron-3-ultra-free")).unwrap();
    assert_eq!(provider.adapter(), "opencode_free");
}

#[test]
fn zen_headers_have_versioned_user_agent_and_unique_canonical_sessions() {
    let first = ZenChat::custom_headers().unwrap();
    let second = ZenChat::custom_headers().unwrap();
    let header = |name: &str| {
        first
            .iter()
            .find(|(key, _)| key == name)
            .unwrap()
            .1
            .as_str()
    };
    assert_eq!(header("User-Agent"), "opencode/1.18.31");
    assert_eq!(header("x-opencode-client"), "desktop");
    let session = header("x-opencode-session");
    assert!(
        regex::Regex::new(r"^ses_[0-9a-f]{12}[0-9A-Za-z]{14}$")
            .unwrap()
            .is_match(session)
    );
    assert_ne!(
        session,
        second
            .iter()
            .find(|(key, _)| key == "x-opencode-session")
            .unwrap()
            .1
    );
}

#[test]
fn fingerprint_tools_preserve_callers_and_never_dispatch_placeholders() {
    use crate::providers::llm::ToolCall;
    let real_tool = ToolDefinition {
        name: "read".into(),
        description: "Read a permitted document".into(),
        parameters: serde_json::json!({"type": "object", "properties": {"id": {"type": "string"}}}),
    };
    let mut request = LlmRequest::from("hello");
    request.tools.push(real_tool.clone());
    let placeholders = add_fingerprint_tools(&mut request);
    assert_eq!(placeholders, ["bash", "glob", "grep"]);
    assert_eq!(request.tools[0], real_tool);
    assert_eq!(request.tools.len(), 4);
    assert!(add_fingerprint_tools(&mut request).is_empty());
    for name in FINGERPRINT_TOOLS {
        let event = Ok(LlmEvent::ToolCall(ToolCall {
            id: "call-1".into(),
            name: name.into(),
            arguments: serde_json::json!({}),
        }));
        assert_eq!(
            reject_placeholder_call(event, &placeholders).is_ok(),
            name == "read"
        );
    }
    assert_eq!(
        reject_placeholder_call(Ok(LlmEvent::TextDelta("OK".into())), &placeholders).unwrap(),
        LlmEvent::TextDelta("OK".into())
    );
    assert!(reject_placeholder_call(Err(LlmError::Failed), &placeholders).is_err());
}
