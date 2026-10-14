//! Model-owned intent only. Executable schemas and permissions come from Rust.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PlanProposal {
    pub steps: Vec<ProposedStep>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ProposedStep {
    Plugin {
        step_key: String,
        plugin_id: String,
        operation: String,
        #[serde(default = "empty_object")]
        arguments: Value,
        #[serde(default)]
        dependencies: Vec<String>,
        #[serde(default = "required")]
        required: bool,
    },
    Llm {
        step_key: String,
        instruction: String,
        #[serde(default)]
        dependencies: Vec<String>,
        #[serde(default = "required")]
        required: bool,
    },
}
fn empty_object() -> Value {
    serde_json::json!({})
}
fn required() -> bool {
    true
}

impl PlanProposal {
    pub(crate) fn parse(raw: &str) -> Result<Self, serde_json::Error> {
        // Strict JSON. Deliberately avoid extracting arbitrary JSON from prose.
        serde_json::from_str(raw)
    }
}
