use super::contract;
use super::contracts::{OPERATIONS, REVISION};
use crate::services::background::plugins::FrozenChoice;
use serde_json::{Value, json};

/// Read-only catalog for the preinstalled Core plugins. These are not rows
/// in plugin_installations and cannot be activated, disabled or uninstalled.
pub(crate) fn bundled_catalog() -> Value {
    let groups: [(&str, &str, &[&str]); 4] = [
        ("core.web", "Core Web", &["web.fetch", "web.extract"]),
        (
            "core.document",
            "Core Document",
            &["document.extract", "document.inspect"],
        ),
        (
            "core.data",
            "Core Data",
            &["data.json_query", "data.transform"],
        ),
        (
            "core.artifact",
            "Core Artifact",
            &["artifact.read_text", "artifact.inspect"],
        ),
    ];
    let items: Vec<Value> = groups
        .iter()
        .map(|(id, name, operations)| {
            let operations: Vec<Value> = operations
                .iter()
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
            json!({"id":id,"name":name,"status":"ready","kind":"built_in",
            "runtime":{"kind":"in_process","language":"rust"},"operations":operations})
        })
        .collect();
    let total = items.len();
    json!({"items":items,"total":total,"operation_total":OPERATIONS.len()})
}
