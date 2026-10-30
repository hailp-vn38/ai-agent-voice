use serde_json::Value;

use crate::app::AppState;
use crate::config::{LlmInstanceConfig, OrchestratorSectionConfig, SecretString};

use super::speech::append_speech_provider_group;
use super::{MuseCatalogOptions, ResolvedLlmProvider, default_orchestrator_provider};

pub async fn resolve_llm_provider_catalog(state: &AppState) -> Vec<ResolvedLlmProvider> {
    let orch_cfg = &state.config.orchestrator;
    let mut out = vec![default_orchestrator_provider(orch_cfg)];

    // 1. Deployment config LLM instances (`config:llm:<key>`)
    collect_config_llm_providers(state, orch_cfg, &mut out);

    // 2. Database LLM providers (`providers` table where `type='llm' AND enabled=1`)
    collect_db_llm_providers(state, orch_cfg, &mut out).await;

    // 3–5. Deployment config, database, and resident TTS providers
    append_speech_provider_group(
        state,
        orch_cfg,
        &mut out,
        "tts",
        state.config.tts.timeout_ms,
        state
            .config
            .providers
            .tts
            .instances
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.adapter().to_owned(),
                    serde_json::to_value(v).unwrap_or(Value::Null),
                )
            })
            .collect(),
        state
            .runtimes
            .tts
            .iter()
            .map(|(k, r)| {
                let prov = r.provider();
                let adapter = prov.adapter();
                if adapter == "unavailable" {
                    return (k.clone(), adapter.to_owned());
                }
                let model_base = prov.model_id().unwrap_or(adapter);
                let voice = r.binding().voice.trim();
                let model_spec = if !voice.is_empty() && voice != "__readiness__" {
                    format!("{model_base}:{voice}")
                } else {
                    model_base.to_owned()
                };
                (k.clone(), format!("{adapter}|{model_spec}"))
            })
            .collect(),
    )
    .await;

    // 6–8. Deployment config, database, and resident ASR providers
    append_speech_provider_group(
        state,
        orch_cfg,
        &mut out,
        "asr",
        state.config.workers.asr.final_timeout_ms,
        state
            .config
            .providers
            .asr
            .instances
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.adapter().to_owned(),
                    serde_json::to_value(v).unwrap_or(Value::Null),
                )
            })
            .collect(),
        state
            .runtimes
            .asr
            .iter()
            .map(|(k, r)| (k.clone(), r.adapter().to_owned()))
            .collect(),
    )
    .await;

    out
}

fn collect_config_llm_providers(
    state: &AppState,
    orch_cfg: &OrchestratorSectionConfig,
    out: &mut Vec<ResolvedLlmProvider>,
) {
    let mut config_keys: Vec<&String> = state.config.providers.llm.instances.keys().collect();
    config_keys.sort();
    for key in config_keys {
        let Some(instance) = state.config.providers.llm.instances.get(key) else {
            continue;
        };
        let provider_ref = format!("config:llm:{key}");
        match instance {
            LlmInstanceConfig::Openai(openai) => {
                let api_key = if openai.gateway_binding.is_some() {
                    None
                } else {
                    (!openai.api_key.expose().trim().is_empty())
                        .then(|| SecretString::new(openai.api_key.expose().to_owned()))
                };
                out.push(ResolvedLlmProvider {
                    provider_ref,
                    name: format!("{key} ({})", openai.model),
                    source: "config",
                    adapter: "openai".to_owned(),
                    model: openai.model.clone(),
                    base_url: openai.base_url.to_string(),
                    api_key,
                    gateway_binding: openai.gateway_binding.clone(),
                    timeout_ms: openai.timeout_ms,
                    max_tokens: openai.max_tokens.unwrap_or(orch_cfg.max_output_tokens),
                    context_limit_tokens: openai
                        .context_limit_tokens
                        .map(|v| v as usize)
                        .unwrap_or(orch_cfg.context_limit_tokens),
                    reasoning_effort: openai.reasoning_effort.clone(),
                    temperature_percent: openai.temperature_percent,
                    tags: crate::providers::llm::effective_llm_tags(openai.tags.as_deref()),
                    muse_options: None,
                });
            }
            LlmInstanceConfig::OpenCodeFree(free) => {
                out.push(ResolvedLlmProvider {
                    provider_ref,
                    name: format!("{key} ({})", free.model),
                    source: "config",
                    adapter: "opencode_free".to_owned(),
                    model: free.model.clone(),
                    base_url: crate::providers::llm::opencode_free::BASE_URL.to_owned(),
                    api_key: Some(SecretString::new(
                        crate::providers::llm::opencode_free::PUBLIC_KEY,
                    )),
                    gateway_binding: None,
                    timeout_ms: free.timeout_ms,
                    max_tokens: free.max_tokens.unwrap_or(orch_cfg.max_output_tokens),
                    context_limit_tokens: free
                        .context_limit_tokens
                        .map(|v| v as usize)
                        .unwrap_or(orch_cfg.context_limit_tokens),
                    reasoning_effort: free.reasoning_effort.clone(),
                    temperature_percent: free.temperature_percent,
                    tags: crate::providers::llm::effective_llm_tags(free.tags.as_deref()),
                    muse_options: None,
                });
            }
            LlmInstanceConfig::LlmGateway(gw) => {
                let base_url = gw
                    .base_url
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| {
                        state
                            .llm_gateway_hub
                            .default_base_url_for_binding(&gw.gateway_binding)
                            .to_owned()
                    });
                let tags = if gw.tags.is_none()
                    && matches!(
                        &gw.gateway_binding,
                        crate::services::llm_gateway::GatewayBinding::Pool(
                            crate::services::llm_gateway::GatewayVendor::Muse
                        )
                    ) {
                    vec![
                        "chat".to_owned(),
                        "vision".to_owned(),
                        "image".to_owned(),
                        "video".to_owned(),
                    ]
                } else {
                    crate::providers::llm::effective_llm_tags(gw.tags.as_deref())
                };
                let muse_options = (gw.account_concurrency.is_some()
                    || gw.cdp_port.is_some()
                    || gw.headless.is_some())
                .then(|| MuseCatalogOptions {
                    account_concurrency: gw.effective_account_concurrency(),
                    cdp_port: gw.effective_cdp_port(),
                    headless: gw.effective_headless(),
                });
                out.push(ResolvedLlmProvider {
                    provider_ref,
                    name: format!("{key} ({})", gw.model),
                    source: "config",
                    adapter: "llm_gateway".to_owned(),
                    model: gw.model.clone(),
                    base_url,
                    api_key: None,
                    gateway_binding: Some(gw.gateway_binding.clone()),
                    timeout_ms: gw.timeout_ms,
                    max_tokens: gw.max_tokens.unwrap_or(orch_cfg.max_output_tokens),
                    context_limit_tokens: gw
                        .context_limit_tokens
                        .map(|v| v as usize)
                        .unwrap_or(orch_cfg.context_limit_tokens),
                    reasoning_effort: gw.reasoning_effort.clone(),
                    temperature_percent: gw.temperature_percent,
                    tags,
                    muse_options,
                });
            }
            #[cfg(feature = "qualification-providers")]
            LlmInstanceConfig::QualificationLlm(_) => {}
        }
    }
}

async fn collect_db_llm_providers(
    state: &AppState,
    orch_cfg: &OrchestratorSectionConfig,
    out: &mut Vec<ResolvedLlmProvider>,
) {
    let Some(database) = &state.database else {
        return;
    };
    let Ok(rows) = sqlx::query_as::<_, (String, String, String, String, Option<String>)>(
        "SELECT key, name, adapter, config_json, credential_json FROM providers WHERE type='llm' AND enabled=1 ORDER BY id",
    )
    .fetch_all(database.pool())
    .await
    else {
        return;
    };

    for (key, name, adapter, config_json, credential_json) in rows {
        let Ok(parsed) = serde_json::from_str::<Value>(&config_json) else {
            continue;
        };
        let model = parsed
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let Some(model) = model else {
            continue;
        };
        let timeout_ms = parsed
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(orch_cfg.request_timeout_ms);
        let max_tokens = parsed
            .get("max_tokens")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(orch_cfg.max_output_tokens);
        let context_limit_tokens = parsed
            .get("context_limit_tokens")
            .and_then(Value::as_u64)
            .and_then(|v| usize::try_from(v).ok())
            .unwrap_or(orch_cfg.context_limit_tokens);
        let reasoning_effort = parsed
            .get("reasoning_effort")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let temperature_percent = parsed
            .get("temperature_percent")
            .and_then(Value::as_u64)
            .and_then(|v| u16::try_from(v).ok());

        let (base_url, api_key, gateway_binding, muse_options) = match adapter.as_str() {
            "openai" => {
                let parsed_binding = parsed
                    .get("gateway_binding")
                    .and_then(Value::as_str)
                    .and_then(|s| crate::services::llm_gateway::GatewayBinding::parse(s).ok());
                let url = parsed
                    .get("base_url")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .or_else(|| {
                        parsed_binding.as_ref().map(|b| {
                            state
                                .llm_gateway_hub
                                .default_base_url_for_binding(b)
                                .to_owned()
                        })
                    });
                let Some(url) = url else {
                    continue;
                };
                let resolved_key = if parsed_binding.is_some() {
                    None
                } else {
                    resolve_db_provider_secret(state, &key, &adapter, credential_json.as_deref())
                };
                (url, resolved_key, parsed_binding, None)
            }
            "llm_gateway" => {
                let Some(binding) = parsed
                    .get("gateway_binding")
                    .and_then(Value::as_str)
                    .and_then(|s| crate::services::llm_gateway::GatewayBinding::parse(s).ok())
                else {
                    continue;
                };
                let url = parsed
                    .get("base_url")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        state
                            .llm_gateway_hub
                            .default_base_url_for_binding(&binding)
                            .to_owned()
                    });
                let account_concurrency = parsed
                    .get("account_concurrency")
                    .and_then(Value::as_u64)
                    .and_then(|v| usize::try_from(v).ok());
                let cdp_port = parsed
                    .get("cdp_port")
                    .and_then(Value::as_u64)
                    .and_then(|v| u16::try_from(v).ok());
                let headless = parsed.get("headless").and_then(Value::as_bool);
                let muse_opts =
                    (account_concurrency.is_some() || cdp_port.is_some() || headless.is_some())
                        .then(|| MuseCatalogOptions {
                            account_concurrency: account_concurrency.unwrap_or(2),
                            cdp_port: cdp_port.unwrap_or(19222),
                            headless: headless.unwrap_or(true),
                        });
                (url, None, Some(binding), muse_opts)
            }
            "opencode_free" => (
                crate::providers::llm::opencode_free::BASE_URL.to_owned(),
                Some(SecretString::new(
                    crate::providers::llm::opencode_free::PUBLIC_KEY,
                )),
                None,
                None,
            ),
            _ => continue,
        };

        let tags = crate::providers::llm::llm_config_tags(&config_json);

        out.push(ResolvedLlmProvider {
            provider_ref: key,
            name,
            source: "database",
            adapter,
            model,
            base_url,
            api_key,
            gateway_binding,
            timeout_ms,
            max_tokens,
            context_limit_tokens,
            reasoning_effort,
            temperature_percent,
            tags,
            muse_options,
        });
    }
}

pub(super) fn resolve_db_provider_secret(
    state: &AppState,
    key: &str,
    adapter: &str,
    credential_json: Option<&str>,
) -> Option<SecretString> {
    let secret_ref = crate::database::credentials::reference(
        &format!("provider:{key}"),
        credential_json,
        crate::database::secrets::provider_secret_env(key, adapter),
    );
    secret_ref
        .and_then(|r| crate::database::secrets::SecretRef::parse(r).ok())
        .and_then(|s| state.secret_resolver.resolve(&s).ok())
        .map(|v| v.expose().trim().to_owned())
        .filter(|s| !s.is_empty())
        .map(SecretString::new)
}
