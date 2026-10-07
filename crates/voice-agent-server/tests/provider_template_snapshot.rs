use std::sync::Arc;
use voice_agent_server::{
    config::EffectiveProviderBindings,
    database::{AdmittedAssignment, AdmittedProviderBinding, DesiredProvider},
    services::provider_runtime::ProviderIdentity,
    session::ConfiguredTemplateProfile,
};
fn assignment() -> AdmittedAssignment {
    AdmittedAssignment {
        template_id: 1,
        template_key: "cold".into(),
        template_name: "Cold".into(),
        language: "vi-VN".into(),
        prompt: "bounded prompt".into(),
        template_enabled: true,
        template_revision: 7,
        is_default: false,
        assignment_enabled: true,
        bindings: ["vad", "asr", "llm", "tts"]
            .into_iter()
            .enumerate()
            .map(|(index, kind)| AdmittedProviderBinding {
                provider_type: kind.into(),
                provider_key: kind.into(),
                provider_enabled: true,
                snapshot: Some(Arc::new(DesiredProvider {
                    id: index as i64 + 1,
                    key: kind.into(),
                    kind: kind.into(),
                    adapter: "fixture".into(),
                    revision: 2,
                    config_json: "{}".into(),
                    secret_ref: None,
                })),
            })
            .collect(),
    }
}
#[test]
fn a_cold_switch_candidate_is_authorized_from_snapshot_without_a_loaded_catalog() {
    let graph = assignment();
    let candidate = ConfiguredTemplateProfile::from_assignment(&graph).unwrap();
    assert_eq!(candidate.template_key(), "cold");
    assert_eq!(candidate.provider_versions().len(), 4);
    drop(graph);
    assert!(
        candidate
            .provider_versions()
            .iter()
            .all(|version| version.revision == 2)
    );
}
#[test]
fn invalid_or_ambiguous_binding_snapshot_never_becomes_a_cold_candidate() {
    let mut graph = assignment();
    graph.bindings[0].snapshot = None;
    assert!(ConfiguredTemplateProfile::from_assignment(&graph).is_err());
    let mut graph = assignment();
    graph.bindings.push(graph.bindings[0].clone());
    assert!(ConfiguredTemplateProfile::from_assignment(&graph).is_err());
    let mut graph = assignment();
    graph.bindings[0].provider_key = "mismatched".into();
    assert!(ConfiguredTemplateProfile::from_assignment(&graph).is_err());
}

#[test]
fn missing_slots_use_deployment_snapshots_without_replacing_explicit_bindings() {
    let mut graph = assignment();
    graph
        .bindings
        .retain(|binding| binding.provider_type == "asr" || binding.provider_type == "tts");
    let defaults = EffectiveProviderBindings {
        vad: "default_vad".into(),
        asr: "default_asr".into(),
        llm: "default_llm".into(),
        tts: "default_tts".into(),
        vision: None,
        speaker: None,
    };
    let deployment = ["vad", "llm"].map(|kind| DesiredProvider {
        id: 0,
        key: format!("default_{kind}"),
        kind: kind.into(),
        adapter: "fixture".into(),
        revision: 1,
        config_json: "{}".into(),
        secret_ref: None,
    });
    let candidate =
        ConfiguredTemplateProfile::from_assignment_with_defaults(&graph, &defaults, &deployment)
            .unwrap();
    let versions = candidate.provider_versions();
    assert_eq!(versions.len(), 4);
    assert_eq!(
        versions
            .iter()
            .filter(|version| matches!(version.identity, ProviderIdentity::Database { .. }))
            .count(),
        2
    );
    assert_eq!(
        versions
            .iter()
            .filter(|version| matches!(version.identity, ProviderIdentity::Deployment { .. }))
            .count(),
        2
    );
    assert!(
        ConfiguredTemplateProfile::from_assignment_with_defaults(
            &graph,
            &defaults,
            &deployment[..1]
        )
        .is_err()
    );
}
