//! Static Core group ownership and the administrator-visible read model.
use super::contract;
use super::contracts::{OPERATIONS, REVISION};
use crate::services::background::plugins::FrozenChoice;
use serde_json::{Value, json};

/// Immutable view of one Core group's mutable policy. Defined beside the
/// built-in catalog, not in a database module, so dependencies flow inward.
#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct CorePluginSetting {
    pub(crate) plugin_id: String,
    pub(crate) enabled: i64,
    pub(crate) revision: i64,
}

#[derive(Debug)]
pub(crate) struct InvalidCorePolicy;

// Only operation membership lives in contracts::OPERATIONS. Group prefixes
// are fixed here; no database or model input can supply executable operations.
const GROUPS: [(&str, &str, &str); 4] = [
    ("core.web", "Core Web", "web."),
    ("core.document", "Core Document", "document."),
    ("core.data", "Core Data", "data."),
    ("core.artifact", "Core Artifact", "artifact."),
];

pub(crate) fn known_group(id: &str) -> bool {
    GROUPS.iter().any(|group| group.0 == id)
}

pub(crate) fn group_for(operation: &str) -> Option<&'static str> {
    if !OPERATIONS.contains(&operation) {
        return None;
    }
    GROUPS
        .iter()
        .find(|(_, _, prefix)| operation.starts_with(prefix))
        .map(|group| group.0)
}

/// Treat a missing, malformed or unknown policy row as corrupt authority,
/// never as permission to silently re-enable a known Core plugin.
fn validated_settings(settings: &[CorePluginSetting]) -> Result<(), InvalidCorePolicy> {
    if settings.len() != GROUPS.len()
        || settings.iter().any(|setting| {
            !known_group(&setting.plugin_id)
                || !matches!(setting.enabled, 0 | 1)
                || setting.revision <= 0
        })
        || GROUPS.iter().any(|group| {
            settings
                .iter()
                .filter(|item| item.plugin_id == group.0)
                .count()
                != 1
        })
    {
        return Err(InvalidCorePolicy);
    }
    if OPERATIONS
        .iter()
        .any(|operation| group_for(operation).is_none())
    {
        return Err(InvalidCorePolicy);
    }
    Ok(())
}

pub(crate) fn enabled_groups(
    settings: &[CorePluginSetting],
) -> Result<Vec<String>, InvalidCorePolicy> {
    validated_settings(settings)?;
    Ok(settings
        .iter()
        .filter(|setting| setting.enabled == 1)
        .map(|setting| setting.plugin_id.clone())
        .collect())
}

/// Render one committed Core group without another SQLite read. A PATCH
/// response therefore cannot fail after its transaction has committed.
pub(crate) fn bundled_item(setting: &CorePluginSetting) -> Option<Value> {
    let (id, name, prefix) = GROUPS.iter().find(|group| group.0 == setting.plugin_id)?;
    let operations: Vec<Value> = OPERATIONS
        .iter()
        .filter(|operation| operation.starts_with(prefix))
        .map(|operation| {
            let choice = FrozenChoice {
                plugin_id: "core".into(),
                operation: (*operation).into(),
                version: REVISION.into(),
                package_digest: String::new(),
                installation_digest: String::new(),
            };
            let description = contract(&choice)
                .ok()
                .and_then(|schema| schema["description"].as_str().map(str::to_owned))
                .unwrap_or_default();
            json!({"name":operation,"description":description,
                "contract_revision":REVISION,"replay":"safe_read"})
        })
        .collect();
    Some(json!({
        "id":id,"name":name,"status":if setting.enabled == 1 {"ready"} else {"disabled"},
        "kind":"built_in","tags":["core"],
        "enabled":setting.enabled == 1,"revision":setting.revision,
        "runtime":{"kind":"in_process","language":"rust"},"operations":operations
    }))
}

pub(crate) fn bundled_catalog(settings: &[CorePluginSetting]) -> Result<Value, InvalidCorePolicy> {
    validated_settings(settings)?;
    let items: Vec<Value> = GROUPS
        .iter()
        .map(|group| {
            let setting = settings
                .iter()
                .find(|setting| setting.plugin_id == group.0)
                .expect("validated Core group");
            bundled_item(setting).expect("validated Core group")
        })
        .collect();
    let total = items.len();
    Ok(json!({"items":items,"total":total,"operation_total":OPERATIONS.len()}))
}
