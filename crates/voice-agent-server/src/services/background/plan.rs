//! Model proposals become executable only after Rust validates the frozen authority and DAG.
use super::{JobError, JobView};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JobPlan {
    pub steps: Vec<PlanStep>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PlanStep {
    pub step_key: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub input_refs: Vec<String>,
    pub instruction: String,
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default = "empty_object")]
    pub arguments: Value,
    pub output_schema: Value,
    #[serde(default = "required")]
    pub required: bool,
}
fn empty_object() -> Value {
    serde_json::json!({})
}
fn required() -> bool {
    true
}

impl JobPlan {
    pub(crate) fn parse(raw: &str, job: &JobView) -> Result<Self, JobError> {
        if raw.len() > 65536 {
            return Err(JobError::InvalidPlan);
        }
        let plan: Self = serde_json::from_str(raw).map_err(|_| JobError::InvalidPlan)?;
        if forbidden(&serde_json::from_str::<Value>(raw).map_err(|_| JobError::InvalidPlan)?)
            || !plan
                .steps
                .iter()
                .any(|step| step.required && step.plugin_id.is_none())
        {
            return Err(JobError::InvalidPlan);
        }
        if plan.steps.is_empty() || plan.steps.len() > job.snapshot.limits.max_steps as usize {
            return Err(JobError::InvalidPlan);
        }
        let mut keys = HashSet::new();
        for step in &plan.steps {
            if step.step_key.is_empty()
                || step.step_key.len() > 64
                || !step
                    .step_key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                || !keys.insert(step.step_key.as_str())
                || step.instruction.trim().is_empty()
                || step.instruction.len() > 4000
                || !step.arguments.is_object()
                || step.arguments.to_string().len() > 16384
                || step.output_schema.to_string().len() > 8192
                || step.dependencies.len() > plan.steps.len()
                || step.input_refs.len() > plan.steps.len()
                || jsonschema::validator_for(&step.output_schema).is_err()
            {
                return Err(JobError::InvalidPlan);
            }
            match (&step.plugin_id, &step.operation) {
                (None, None) => {}
                (Some(plugin), Some(operation))
                    if job.plugin_choices.as_array().is_some_and(|choices| {
                        choices.iter().any(|choice| {
                            choice["plugin_id"] == *plugin && choice["operation"] == *operation
                        })
                    }) => {}
                _ => return Err(JobError::CapabilityUnavailable),
            }
        }
        let mut visited = HashSet::new();
        let mut pending: HashMap<_, _> = plan
            .steps
            .iter()
            .map(|step| (step.step_key.as_str(), step))
            .collect();
        for step in &plan.steps {
            let dependencies: HashSet<_> = step.dependencies.iter().collect();
            if dependencies.len() != step.dependencies.len()
                || step
                    .dependencies
                    .iter()
                    .any(|key| !keys.contains(key.as_str()) || key == &step.step_key)
                || step
                    .input_refs
                    .iter()
                    .any(|key| !dependencies.contains(key))
            {
                return Err(JobError::InvalidPlan);
            }
        }
        while !pending.is_empty() {
            let ready: Vec<_> = pending
                .iter()
                .filter(|(_, step)| {
                    step.dependencies
                        .iter()
                        .all(|key| visited.contains(key.as_str()))
                })
                .map(|(key, _)| *key)
                .collect();
            if ready.is_empty() {
                return Err(JobError::InvalidPlan);
            }
            for key in ready {
                pending.remove(key);
                visited.insert(key);
            }
        }
        Ok(plan)
    }
}
pub(crate) fn forbidden(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(key, value)| {
            key == "document_refs"
                || (key == "$ref"
                    && value
                        .as_str()
                        .is_some_and(|reference| !reference.starts_with('#')))
                || forbidden(value)
        }),
        Value::Array(values) => values.iter().any(forbidden),
        _ => false,
    }
}
