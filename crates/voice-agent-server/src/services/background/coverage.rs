use super::{JobError, JobView, plan::JobPlan};
use serde_json::{Value, json};
use std::collections::HashMap;

pub(super) fn terminal_result<'a>(
    job: &JobView,
    plan: &JobPlan,
    results: &HashMap<String, Value>,
    failures: &HashMap<String, String>,
) -> Result<(&'a str, Value), JobError> {
    let mut sources = std::collections::HashSet::new();
    let mut capabilities = std::collections::HashSet::new();
    for step in &plan.steps {
        let Some(plugin) = &step.plugin_id else {
            continue;
        };
        let Some(result) = results.get(&step.step_key) else {
            continue;
        };
        let choice = job
            .plugin_choices
            .as_array()
            .and_then(|choices| {
                choices.iter().find(|choice| {
                    choice["plugin_id"] == *plugin
                        && choice["operation"] == *step.operation.as_ref().unwrap()
                })
            })
            .ok_or(JobError::CapabilityUnavailable)?;
        if let Some(provenance) = result.get("provenance").and_then(Value::as_array) {
            if provenance.len() > 32 {
                return Err(JobError::InvalidPlan);
            }
            for evidence in provenance {
                let Some(source) = evidence["source"].as_str() else {
                    continue;
                };
                let usable = evidence["urls"].as_array().is_some_and(|urls| {
                    !urls.is_empty()
                        && urls.len() <= 16
                        && urls.iter().all(|url| {
                            url.as_str().is_some_and(|url| {
                                url.len() <= 2048
                                    && (url.starts_with("https://") || url.starts_with("http://"))
                            })
                        })
                }) || evidence["records"].as_array().is_some_and(|records| {
                    !records.is_empty()
                        && records.len() <= 16
                        && records.iter().all(|record| {
                            record.as_str().is_some_and(|record| {
                                !record.trim().is_empty() && record.len() <= 2048
                            })
                        })
                });
                if usable
                    && choice["sources"]
                        .as_array()
                        .is_some_and(|declared| declared.iter().any(|value| value == source))
                {
                    sources.insert(source.to_owned());
                    if let Some(provided) = evidence["capabilities"].as_array() {
                        for capability in provided {
                            if let Some(capability) = capability.as_str()
                                && choice["provides"].as_array().is_some_and(|declared| {
                                    declared.iter().any(|value| value == capability)
                                })
                            {
                                capabilities.insert(capability.to_owned());
                            }
                        }
                    }
                }
            }
        }
    }
    let missing_sources: Vec<_> = job
        .request
        .required_sources
        .iter()
        .filter(|source| !sources.contains(*source))
        .collect();
    let missing_capabilities: Vec<_> = job
        .request
        .required_capabilities
        .iter()
        .filter(|capability| !capabilities.contains(*capability))
        .collect();
    let required_failed = plan
        .steps
        .iter()
        .any(|step| step.required && failures.contains_key(&step.step_key));
    let satisfied =
        missing_sources.is_empty() && missing_capabilities.is_empty() && !required_failed;
    let status = if satisfied {
        "succeeded"
    } else if job.request.allow_partial && !job.artifacts.is_empty() && !results.is_empty() {
        "partial_success"
    } else {
        "failed"
    };
    let output = json!({"steps":results,"failed_steps":failures,"missing_sources":missing_sources,"missing_capabilities":missing_capabilities,"coverage_basis":if job.request.required_sources.is_empty() && job.request.required_capabilities.is_empty() {"goal_only_unverified"}else{"structured_provenance"}});
    if output.to_string().len() > 65536 {
        return Err(JobError::CapacityUnavailable);
    }
    Ok((status, output))
}
