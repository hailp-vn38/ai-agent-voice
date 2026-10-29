use serde_json::Value;

use crate::app::AppState;
use crate::config::OrchestratorSectionConfig;

use super::ResolvedLlmProvider;
use super::discovery::resolve_db_provider_secret;

pub(super) async fn append_speech_provider_group(
    state: &AppState,
    orch_cfg: &OrchestratorSectionConfig,
    out: &mut Vec<ResolvedLlmProvider>,
    kind: &str,
    timeout_ms: u64,
    mut config_entries: Vec<(String, String, Value)>,
    mut runtime_entries: Vec<(String, String)>,
) {
    config_entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, adapter, serialized) in config_entries {
        let (model, base_url) = extract_speech_model_and_url(kind, &adapter, &serialized);
        out.push(ResolvedLlmProvider {
            provider_ref: format!("config:{kind}:{key}"),
            name: format!("{key} ({model})"),
            source: "config",
            adapter,
            model,
            base_url,
            api_key: None,
            gateway_binding: None,
            timeout_ms,
            max_tokens: orch_cfg.max_output_tokens,
            context_limit_tokens: orch_cfg.context_limit_tokens,
            reasoning_effort: None,
            temperature_percent: None,
            tags: vec![kind.to_owned()],
            muse_options: None,
        });
    }

    if let Some(database) = &state.database
        && let Ok(rows) = sqlx::query_as::<_, (String, String, String, String, Option<String>)>(
            "SELECT key, name, adapter, config_json, credential_json FROM providers WHERE type=?1 AND enabled=1 ORDER BY id",
        )
        .bind(kind)
        .fetch_all(database.pool())
        .await
    {
        for (key, name, adapter, config_json, credential_json) in rows {
            if out.iter().any(|p| p.provider_ref == key) {
                continue;
            }
            let parsed = serde_json::from_str::<Value>(&config_json).unwrap_or(Value::Null);
            let (model, base_url) = extract_speech_model_and_url(kind, &adapter, &parsed);
            let api_key = resolve_db_provider_secret(
                state,
                &key,
                &adapter,
                credential_json.as_deref(),
            );
            out.push(ResolvedLlmProvider {
                provider_ref: key,
                name,
                source: "database",
                adapter,
                model,
                base_url,
                api_key,
                gateway_binding: None,
                timeout_ms,
                max_tokens: orch_cfg.max_output_tokens,
                context_limit_tokens: orch_cfg.context_limit_tokens,
                reasoning_effort: None,
                temperature_percent: None,
                tags: vec![kind.to_owned()],
                muse_options: None,
            });
        }
    }

    runtime_entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, adapter) in runtime_entries {
        if adapter == "unavailable" {
            continue;
        }
        let config_ref = format!("config:{kind}:{key}");
        if out
            .iter()
            .any(|p| p.provider_ref == key || p.provider_ref == config_ref)
        {
            continue;
        }
        out.push(ResolvedLlmProvider {
            provider_ref: key.clone(),
            name: format!("{key} ({adapter})"),
            source: "runtime",
            adapter: adapter.clone(),
            model: adapter,
            base_url: String::new(),
            api_key: None,
            gateway_binding: None,
            timeout_ms,
            max_tokens: orch_cfg.max_output_tokens,
            context_limit_tokens: orch_cfg.context_limit_tokens,
            reasoning_effort: None,
            temperature_percent: None,
            tags: vec![kind.to_owned()],
            muse_options: None,
        });
    }
}

fn extract_speech_model_and_url(kind: &str, adapter: &str, parsed: &Value) -> (String, String) {
    let raw_model = if kind == "asr" {
        parsed
            .get("model")
            .or_else(|| parsed.get("model_dir"))
            .or_else(|| parsed.get("encoder"))
    } else {
        parsed.get("model").or_else(|| parsed.get("voice"))
    };
    let model = raw_model
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            if kind == "asr" {
                std::path::Path::new(s)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(s)
            } else {
                s
            }
        })
        .unwrap_or(adapter)
        .to_owned();
    let base_url = parsed
        .get("url")
        .or_else(|| parsed.get("base_url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    (model, base_url)
}
