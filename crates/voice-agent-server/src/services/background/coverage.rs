use super::{JobError, JobView};
use serde_json::{Value, json};
use std::collections::HashMap;

pub(super) fn terminal_result<'a>(
    job: &JobView,
    results: &HashMap<String, Value>,
    failures: &HashMap<String, String>,
) -> Result<(&'a str, Value), JobError> {
    let mut sources = std::collections::HashSet::new();
    let mut capabilities = std::collections::HashSet::new();
    for step in job.steps.iter().filter(|step| step.agent_run_id.is_some()) {
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
    let satisfied = missing_sources.is_empty() && missing_capabilities.is_empty();
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


/// Narrow requirement for a single-URL text/article summary.
pub(super) fn requested_summary_url(request: &super::model::JobRequest) -> Option<String> {
    if !matches!(request.output_format.as_str(), "summary" | "text" | "markdown") {
        return None;
    }
    let lower = request.goal.to_lowercase();
    if !lower.contains("tóm tắt") && !lower.contains("summariz") {
        return None;
    }
    let re = regex::Regex::new(r#"https?://[^\s"'<>]+"#).ok()?;
    let urls: Vec<String> = re
        .find_iter(&request.goal)
        .filter_map(|found| canonical_source_url(
            found.as_str().trim_end_matches(&['.', ',', ';', '!', '?', ')', ']'][..])
        ))
        .collect();
    // Do not invent completeness rules for multi-source research prompts.
    (urls.len() == 1).then(|| urls[0].clone())
}

fn canonical_source_url(raw: &str) -> Option<String> {
    let mut url = url::Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    url.set_fragment(None);
    Some(url.to_string())
}

/// Prove the entire extracted text was read from persisted successful core
/// Steps: offsets must cover 0..end without gaps, the server-issued
/// provenance must match the requested URL, and no body cap may be reached.
pub(super) fn completed_web_sources(job: &JobView, session_run_ids: Option<&[String]>) -> Vec<String> {
    let mut urls = std::collections::BTreeSet::new();
    for step in job.steps.iter().filter(|step| {
        step.status == "succeeded"
            && step.plugin_id.as_deref() == Some("core")
            && step.operation.as_deref() == Some("web.extract")
            && session_run_ids.is_none_or(|ids| step.agent_run_id.as_deref().is_some_and(|id| ids.iter().any(|run| run == id)))
    }) {
        if let Some(url) = step.arguments["url"].as_str().and_then(canonical_source_url)
            && extraction_is_complete(&job.steps, step, &url, session_run_ids)
        {
            urls.insert(url);
        }
    }
    urls.into_iter().take(16).collect()
}

fn extraction_is_complete(
    steps: &[super::model::JobStepView],
    candidate: &super::model::JobStepView,
    canonical_url: &str,
    session_run_ids: Option<&[String]>,
) -> bool {
    let mode = candidate.arguments["mode"].as_str().unwrap_or("article");
    let mut offset = 0u64;
    for _ in 0..steps.len() {
        let Some(step) = steps.iter().find(|step| {
            step.status == "succeeded"
                && step.plugin_id.as_deref() == Some("core")
                && step.operation.as_deref() == Some("web.extract")
                && session_run_ids.is_none_or(|ids| step.agent_run_id.as_deref().is_some_and(|id| ids.iter().any(|run| run == id)))
                && step.arguments["mode"].as_str().unwrap_or("article") == mode
                && step.arguments["offset"].as_u64().unwrap_or(0) == offset
                && step.arguments["url"].as_str()
                    .and_then(canonical_source_url).as_deref() == Some(canonical_url)
                && verified_extract_result(step, canonical_url)
        }) else {
            return false;
        };
        let result = step.result.as_ref().expect("verified result present");
        if result["body_limit_reached"] == true {
            return false;
        }
        if result["truncated"] == false {
            return true;
        }
        let Some(next) = result["next_offset"].as_u64() else {
            return false;
        };
        let length = result["text"].as_str().map(|text| text.chars().count() as u64)
            .unwrap_or(0);
        if next != offset.saturating_add(length) || next <= offset {
            return false;
        }
        offset = next;
    }
    false
}

fn verified_extract_result(step: &super::model::JobStepView, url: &str) -> bool {
    let Some(result) = step.result.as_ref() else {
        return false;
    };
    if !result["status"].as_u64().is_some_and(|status| (200..300).contains(&status))
        || !result["text"].as_str().is_some_and(|text|
            !text.trim().is_empty() && text.chars().count() <= 6000)
        // The model cannot verify bytes hidden behind a bounded Tool Result.
        // Larger extracts must be re-requested in <=6000-character pages.
        || result["truncated"].as_bool().is_none()
        || result["url"].as_str().and_then(canonical_source_url).as_deref() != Some(url)
    {
        return false;
    }
    result["provenance"].as_array().is_some_and(|records| {
        records.iter().any(|entry| {
            entry["source"] == "web"
                && entry["capabilities"].as_array().is_some_and(|capabilities|
                    capabilities.iter().any(|capability| capability == "web_extract"))
                && entry["urls"].as_array().is_some_and(|urls|
                    urls.iter().any(|candidate|
                        candidate.as_str().and_then(canonical_source_url).as_deref() == Some(url)))
        })
    })
}

#[cfg(test)]
mod source_tests {
    use super::*;
    use super::super::model::{JobRequest, JobStepView};

    fn step(offset: u64, text: &str, truncated: bool, next: Option<u64>) -> JobStepView {
        let url = "https://news.example/article";
        JobStepView {
            step_key: format!("step_{offset}"),
            agent_run_id: Some("child_1".into()),
            agent_turn_id: None,
            status: "succeeded".into(),
            plugin_id: Some("core".into()),
            operation: Some("web.extract".into()),
            error_code: None,
            attempt_state: Some("finished".into()),
            instruction: String::new(),
            required: true,
            dependencies: vec![],
            input_refs: vec![],
            arguments: json!({"url":url,"mode":"article","offset":offset}),
            output_schema: json!({}),
            result: Some(json!({
                "url":url,"text":text,"status":200,"truncated":truncated,
                "next_offset":next,"body_limit_reached":false,
                "provenance":[{"source":"web","capabilities":["web_extract"],"urls":[url]}]
            })),
        }
    }

    #[test]
    fn recognizes_single_url_summary_even_with_trailing_quotes() {
        let request = JobRequest {
            goal: r#"tóm tắt: https://news.example/article.html"."#.into(),
            output_format: "text".into(),
            allow_partial: false,
            required_sources: vec![],
            required_capabilities: vec![],
        };
        assert_eq!(requested_summary_url(&request).as_deref(),
            Some("https://news.example/article.html"));
    }

    #[test]
    fn does_not_accept_navigation_chunks_with_missing_offsets() {
        let steps = vec![
            step(0, "menu", true, Some(4)),
            step(1000, "article fragment", true, Some(1016)),
        ];
        assert!(!extraction_is_complete(&steps, &steps[0], "https://news.example/article", None));
    }

    #[test]
    fn complete_contiguous_article_pages_have_source_proof() {
        let steps = vec![
            step(0, "INTRO", true, Some(5)),
            step(5, "ARTICLE", false, None),
        ];
        assert!(extraction_is_complete(&steps, &steps[0], "https://news.example/article", None));
        let mut bad = steps.clone();
        bad[1].result.as_mut().unwrap()["provenance"] = json!([]);
        assert!(!extraction_is_complete(&bad, &bad[0], "https://news.example/article", None));
    }
}
