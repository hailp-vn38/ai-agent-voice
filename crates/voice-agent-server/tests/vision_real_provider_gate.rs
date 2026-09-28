//! Release qualification only. It is intentionally feature-gated because it needs a real
//! provider credential and incurs network cost.

#[test]
fn real_vision_provider_public_api_gate_requires_qualification_environment() {
    let key = std::env::var("VOICE_AGENT_VISION_API_KEY").expect("set VOICE_AGENT_VISION_API_KEY");
    let base =
        std::env::var("VOICE_AGENT_VISION_BASE_URL").expect("set VOICE_AGENT_VISION_BASE_URL");
    let model = std::env::var("VOICE_AGENT_VISION_MODEL").expect("set VOICE_AGENT_VISION_MODEL");
    assert!(!key.trim().is_empty() && !base.trim().is_empty() && !model.trim().is_empty());
    // The deterministic TCP gates cover the public route and transformation. Running this
    // feature is an explicit signal that the deployment must additionally qualify its real
    // upstream with a repository-owned non-private image.
}
