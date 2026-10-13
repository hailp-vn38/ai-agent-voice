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


/// One diagnostic vocabulary for proposal repair and admin-safe Job events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlanErrorCategory {
    Syntax,
    Dag,
    Contract,
    Forbidden,
    Limit,
}
#[derive(Clone, Debug)]
pub(crate) struct PlanValidationError {
    pub category: PlanErrorCategory,
    pub code: &'static str,
    pub path: String,
    pub message: &'static str,
}
impl PlanValidationError {
    pub(crate) fn new(category: PlanErrorCategory, code: &'static str, path: impl Into<String>, message: &'static str) -> Self {
        Self { category, code, path: path.into(), message }
    }
    pub(crate) fn repairable(&self) -> bool {
        matches!(self.category, PlanErrorCategory::Syntax | PlanErrorCategory::Dag | PlanErrorCategory::Contract)
    }
    pub(crate) fn feedback(&self) -> serde_json::Value {
        serde_json::json!({"code":self.code,"path":self.path,"message":self.message})
    }
}

/// Only Rust materializes executable output schemas from pinned, frozen contracts.
/// No I/O or mutable authority lives behind this interface.
pub(crate) struct PlanCompiler;
impl PlanCompiler {
    pub(crate) fn compile(
        proposal: super::proposal::PlanProposal,
        job: &JobView,
        contracts: &HashMap<(String, String), Value>,
        previous: Option<&JobPlan>,
    ) -> Result<JobPlan, PlanValidationError> {
        use super::proposal::ProposedStep;
        fn violation(category: PlanErrorCategory, code: &'static str, path: impl Into<String>, message: &'static str) -> PlanValidationError {
            PlanValidationError::new(category,code,path,message)
        }
        let proposed_json = serde_json::to_value(&proposal).map_err(|_| violation(
            PlanErrorCategory::Syntax,"proposal_not_json","$","Proposal cannot be encoded"))?;
        if forbidden(&proposed_json) {
            return Err(violation(PlanErrorCategory::Forbidden,"forbidden_field","$","Proposal contains a forbidden reference"));
        }
        if proposal.steps.is_empty() || proposal.steps.len() > job.snapshot.limits.max_steps as usize {
            return Err(violation(PlanErrorCategory::Limit,"step_count","$.steps","Invalid number of proposed steps"));
        }
        let replan = previous.is_some();
        let mut steps = previous.map_or_else(Vec::new, |old| old.steps.clone());
        let mut has_new_synthesis = false;
        for (index, proposed) in proposal.steps.into_iter().enumerate() {
            let path = format!("$.steps[{index}]");
            let step = match proposed {
                ProposedStep::Plugin {step_key, plugin_id, operation, arguments, dependencies, required} => {
                    let contract = contracts.get(&(plugin_id.clone(),operation.clone())).ok_or_else(|| violation(
                        PlanErrorCategory::Contract,"unknown_operation",format!("{path}.operation"),"Operation is not in frozen choices"))?;
                    let input_schema = contract.get("input_schema").ok_or_else(|| violation(
                        PlanErrorCategory::Contract,"missing_contract_schema",format!("{path}.operation"),"Frozen contract lacks input schema"))?;
                    let validator = jsonschema::validator_for(input_schema).map_err(|_| violation(
                        PlanErrorCategory::Forbidden,"invalid_frozen_schema",format!("{path}.operation"),"Frozen input schema is invalid"))?;
                    if !arguments.is_object() || !validator.is_valid(&arguments) {
                        return Err(violation(PlanErrorCategory::Contract,"plugin_arguments_invalid",format!("{path}.arguments"),"Arguments do not match frozen input schema"));
                    }
                    let output_schema = contract.get("output_schema").ok_or_else(|| violation(
                        PlanErrorCategory::Forbidden,"invalid_frozen_schema",format!("{path}.operation"),"Frozen output schema missing"))?.clone();
                    let instruction = contract.get("description").and_then(Value::as_str)
                        .filter(|text| !text.trim().is_empty() && text.len() <= 4000)
                        .map(str::to_owned).unwrap_or_else(|| format!("Execute {plugin_id}.{operation}"));
                    PlanStep {step_key, dependencies, input_refs:Vec::new(), instruction,
                        plugin_id:Some(plugin_id),operation:Some(operation),arguments,output_schema,required}
                }
                ProposedStep::Llm {step_key,instruction,dependencies,required} => {
                    has_new_synthesis |= required;
                    PlanStep {step_key, input_refs:dependencies.clone(), dependencies, instruction,
                        plugin_id:None,operation:None,arguments:serde_json::json!({}),
                        output_schema:serde_json::json!({"type":"object","required":["summary"],"properties":{"summary":{"type":"string"}},"additionalProperties":false}),required}
                }
            };
            if steps.iter().any(|old| old.step_key == step.step_key) {
                return Err(violation(PlanErrorCategory::Dag,"duplicate_step_key",format!("{path}.step_key"),"Step key already exists"));
            }
            if replan && step.plugin_id.is_some() && steps.iter().any(|old|
                old.plugin_id == step.plugin_id && old.operation == step.operation
            ) {
                return Err(violation(PlanErrorCategory::Forbidden,"operation_already_attempted",format!("{path}.operation"),"Replan cannot repeat a plugin operation"));
            }
            steps.push(step);
        }
        if !has_new_synthesis {
            return Err(violation(PlanErrorCategory::Dag,"missing_synthesis","$.steps","At least one required LLM synthesis step is required"));
        }
        let keys:HashSet<_> = steps.iter().map(|s|s.step_key.as_str()).collect();
        for step in &steps {
            if step.dependencies.iter().any(|dependency|!keys.contains(dependency.as_str())) {
                return Err(violation(PlanErrorCategory::Dag,"unknown_dependency","$.steps","A step dependency does not exist"));
            }
        }
        let plan=JobPlan{steps};
        // Reuse the existing authoritative DAG and policy validator.
        JobPlan::parse(&serde_json::to_string(&plan).map_err(|_|
            violation(PlanErrorCategory::Syntax,"proposal_not_json","$","Cannot encode executable plan")
        )?, job).map_err(|error| violation(
            if error == JobError::CapabilityUnavailable {PlanErrorCategory::Contract}else{PlanErrorCategory::Dag},
            if error == JobError::CapabilityUnavailable {"unknown_operation"}else{"invalid_dag"},
            "$.steps","Executable plan failed validation"))?;
        Ok(plan)
    }
}
