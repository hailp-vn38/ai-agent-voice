use super::*;
use crate::database::providers::Provider;

pub(super) fn provider_response(
    provider: Provider,
    runtime_snapshot: Option<&crate::providers::DatabaseRuntimeSnapshot>,
) -> Value {
    let runtime = runtime_snapshot
        .map(|snapshot| snapshot.runtime_state(&provider.key, provider.revision))
        .unwrap_or_else(|| crate::providers::DatabaseRuntimeState {
            provider_id: provider.id,
            desired_revision: provider.revision,
            status: crate::providers::DatabaseRuntimeStatus::NotLoaded,
            failure: None,
        });
    let matches_desired = matches!(
        runtime.status,
        crate::providers::DatabaseRuntimeStatus::Loaded
    ) && runtime.desired_revision == provider.revision;
    let runtime_status = match runtime.status {
        crate::providers::DatabaseRuntimeStatus::NotLoaded => "not_loaded",
        crate::providers::DatabaseRuntimeStatus::Unavailable => "unavailable",
        crate::providers::DatabaseRuntimeStatus::Loaded => "loaded",
    };
    let credential_env = crate::database::secrets::provider_secret_env_for_config(
        &provider.key,
        &provider.adapter,
        &provider.config_json,
    );
    let credential = crate::database::credentials::metadata(provider.credential_json.as_deref());
    let mut response = serde_json::to_value(provider).expect("Provider is serializable");
    response["credential"] = credential;
    response["source"] = Value::String("database".into());
    let object = response
        .as_object_mut()
        .expect("Provider serializes to object");
    object.insert(
        "runtime_status".into(),
        Value::String(runtime_status.into()),
    );
    object.insert(
        "runtime_matches_desired".into(),
        Value::Bool(matches_desired),
    );
    object.insert("requires_restart".into(), Value::Bool(!matches_desired));
    object.insert(
        "credential_env".into(),
        serde_json::to_value(credential_env).unwrap(),
    );
    response
}

pub(super) fn managed_provider_response(provider: Provider, state: &AppState) -> Value {
    if provider.id == 0 {
        let reference = crate::providers::config_catalog::ProviderRef::parse(&provider.key, None)
            .expect("catalog reference");
        let runtime = state.provider_runtime_manager.as_ref().map(|manager| {
            manager.inspect_version(&crate::services::provider_runtime::ProviderVersion {
                identity: crate::services::provider_runtime::ProviderIdentity::Deployment {
                    kind: reference.kind,
                    key: reference.key,
                },
                revision: provider.revision,
            })
        });
        use crate::services::provider_runtime::RuntimeState;
        let ready = runtime
            .as_ref()
            .is_some_and(|r| r.desired_state == RuntimeState::Ready);
        let status = match runtime.as_ref().map(|r| r.desired_state) {
            Some(RuntimeState::Ready) => "loaded",
            Some(RuntimeState::Failed | RuntimeState::Quarantined) => "unavailable",
            _ => "not_loaded",
        };
        let mut response = serde_json::to_value(provider).expect("Provider is serializable");
        response["source"] = Value::String("config".into());
        response["credential_env"] = Value::Null;
        response["credential"] = crate::database::credentials::metadata(None);
        response["runtime_status"] = Value::String(status.into());
        response["runtime_matches_desired"] = Value::Bool(ready);
        response["requires_restart"] = Value::Bool(false);
        if let Some(runtime) = runtime {
            response["runtime"] = serde_json::to_value(runtime).expect("runtime is serializable");
        }
        return response;
    }
    let Some(manager) = &state.provider_runtime_manager else {
        return provider_response(provider, state.database_runtime_snapshot.as_deref());
    };
    let mut runtime = manager.inspect(provider.id, provider.revision);
    runtime.can_prepare &= provider.enabled != 0;
    use crate::services::provider_runtime::RuntimeState;
    let ready = runtime.desired_state == RuntimeState::Ready;
    let status = match runtime.desired_state {
        RuntimeState::Ready => "loaded",
        RuntimeState::Failed | RuntimeState::Quarantined => "unavailable",
        _ => "not_loaded",
    };
    let credential_env = crate::database::secrets::provider_secret_env_for_config(
        &provider.key,
        &provider.adapter,
        &provider.config_json,
    );
    let credential = crate::database::credentials::metadata(provider.credential_json.as_deref());
    let mut response = serde_json::to_value(provider).expect("Provider is serializable");
    response["credential"] = credential;
    response["source"] = Value::String("database".into());
    response["credential_env"] = serde_json::to_value(credential_env).unwrap();
    response["runtime_status"] = Value::String(status.into());
    response["runtime_matches_desired"] = Value::Bool(ready);
    response["requires_restart"] = Value::Bool(false);
    response["runtime"] =
        serde_json::to_value(runtime).expect("runtime inspection is serializable");
    response
}

pub(super) fn adapter_matches_kind(kind: &str, adapter: &str) -> bool {
    crate::providers::admin_provider_adapter_matches_kind(kind, adapter)
}

/// Mints the immutable identity for a new provider as `{provider_type}_{uuid32}`.
///
/// The key never derives from the display name: names repeat, get renamed and carry no slug
/// rules, so a random suffix keeps every create collision-free without a retry loop. The result
/// satisfies the `valid_key` rules a client-supplied key once had to, so existing rows, URLs and
/// template bindings keep working unchanged. `kind` must already be a compiled provider
/// type, which is what keeps the prefix lowercase.
pub(super) fn generate_provider_key(kind: &str) -> String {
    format!("{kind}_{}", Uuid::new_v4().simple())
}

#[cfg(test)]
mod provider_runtime_tests {
    use super::*;
    use crate::providers::{DatabaseRuntimeSnapshot, DatabaseRuntimeState, DatabaseRuntimeStatus};

    #[test]
    fn a_generated_key_is_prefixed_by_type_and_never_repeats() {
        let key = generate_provider_key("llm");
        assert!(key.starts_with("llm_"), "{key}");
        assert!(valid_key(&key), "{key}");
        assert_eq!(key.len(), "llm_".len() + 32, "{key}");
        assert_ne!(key, generate_provider_key("llm"));
        assert_ne!(key, generate_provider_key("tts"));
    }

    #[test]
    fn loaded_runtime_becomes_stale_after_desired_revision_changes() {
        let provider = Provider {
            id: 11,
            key: "llm-main".into(),
            name: "LLM".into(),
            kind: "llm".into(),
            adapter: "openai".into(),
            config_json: "{}".into(),
            enabled: 1,
            revision: 4,
            created_at: 0,
            updated_at: 0,
            credential_json: None,
        };
        let snapshot = DatabaseRuntimeSnapshot::from_states([(
            "llm-main".into(),
            DatabaseRuntimeState {
                provider_id: 11,
                desired_revision: 3,
                status: DatabaseRuntimeStatus::Loaded,
                failure: None,
            },
        )]);
        let response = provider_response(provider, Some(&snapshot));
        assert_eq!(response["runtime_status"], "loaded");
        assert_eq!(response["runtime_matches_desired"], false);
        assert_eq!(response["requires_restart"], true);
    }
}
