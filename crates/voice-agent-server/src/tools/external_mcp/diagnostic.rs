//! Manual observations; never publish Agent authority or invoke discovered tools.
use super::ExternalMcpManager;
use crate::{
    database::{
        AdmittedMcpServer, Database, external_mcp_policy,
        mcp_servers::mcp_by,
        secrets::{SecretResolver, SecretValue},
    },
    services::test_credentials::{InlineCredential, SavedCredential},
};
use std::{sync::Arc, time::Instant};

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum McpAuth {
    None {},
    Bearer {},
    Header { header_name: String },
}
pub(crate) fn auth_parts(auth: McpAuth) -> Option<(String, Option<String>)> {
    match auth {
        McpAuth::None {} => Some(("none".into(), None)),
        McpAuth::Bearer {} => Some(("bearer".into(), None)),
        McpAuth::Header { header_name }
            if !header_name.is_empty()
                && header_name == header_name.to_ascii_lowercase()
                && header_name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && super::transport::canonical_static_headers(
                    &serde_json::json!({header_name.clone(): "x"}),
                )
                .is_ok() =>
        {
            Some(("header".into(), Some(header_name)))
        }
        _ => None,
    }
}
fn default_connect() -> u64 {
    5000
}
fn default_request() -> u64 {
    30000
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct McpProbeConfig {
    pub key: String,
    pub url: String,
    pub auth: McpAuth,
    pub api_key: Option<SecretValue>,
    pub saved_credential: Option<SavedCredential>,
    #[serde(default = "default_connect")]
    pub connect_timeout_ms: u64,
    #[serde(default = "default_request")]
    pub request_timeout_ms: u64,
}
pub(crate) enum McpProbeSource {
    Saved(String),
    Draft(McpProbeConfig),
}
pub(crate) struct McpProbeError {
    pub status: http::StatusCode,
    pub code: &'static str,
}
impl McpProbeError {
    fn invalid(code: &'static str) -> Self {
        Self {
            status: http::StatusCode::BAD_REQUEST,
            code,
        }
    }
    fn database(error: sqlx::Error) -> Self {
        Self {
            status: if matches!(error, sqlx::Error::RowNotFound) {
                http::StatusCode::NOT_FOUND
            } else {
                http::StatusCode::SERVICE_UNAVAILABLE
            },
            code: if matches!(error, sqlx::Error::RowNotFound) {
                "not_found"
            } else {
                "database_unavailable"
            },
        }
    }
}
pub(crate) struct McpDiagnosticRunner<'a> {
    pub database: &'a Database,
    pub manager: Arc<ExternalMcpManager>,
    pub secrets: Arc<dyn SecretResolver>,
    pub network: &'a crate::config::ExternalMcpNetworkConfig,
}
impl McpDiagnosticRunner<'_> {
    pub async fn probe(
        &self,
        source: McpProbeSource,
        discover: bool,
    ) -> Result<serde_json::Value, McpProbeError> {
        if !self.manager.gate.is_open() {
            return Err(McpProbeError {
                status: http::StatusCode::SERVICE_UNAVAILABLE,
                code: "server_is_shutting_down",
            });
        }
        let permit = self
            .manager
            .probe_permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| McpProbeError {
                status: http::StatusCode::TOO_MANY_REQUESTS,
                code: "mcp_test_busy",
            })?;
        let started = Instant::now();
        let (server, secrets, source) = match source {
            McpProbeSource::Saved(key) => {
                let row = mcp_by(self.database, &key)
                    .await
                    .map_err(McpProbeError::database)?;
                let secret_ref = crate::database::credentials::reference(
                    &format!("mcp:{}", row.key),
                    row.credential_json.as_deref(),
                    crate::database::secrets::mcp_secret_env(&row.key, &row.auth_type),
                );
                (
                    AdmittedMcpServer {
                        id: row.id,
                        key: row.key,
                        url: row.url,
                        headers_json: "{}".into(),
                        auth_type: row.auth_type,
                        auth_header_name: row.auth_header_name,
                        secret_ref,
                        connect_timeout_ms: row.connect_timeout_ms,
                        request_timeout_ms: row.request_timeout_ms,
                        revision: row.revision,
                    },
                    self.secrets.clone(),
                    "saved",
                )
            }
            McpProbeSource::Draft(config) => {
                if config.key.is_empty()
                    || config.key.len() > 64
                    || !config.key.as_bytes()[0].is_ascii_lowercase()
                    || !config
                        .key
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                    || !external_mcp_policy::valid_desired_url(&config.url, self.network)
                    || !(1..=60_000).contains(&config.connect_timeout_ms)
                    || !(1..=120_000).contains(&config.request_timeout_ms)
                {
                    return Err(McpProbeError::invalid("validation_failed"));
                }
                let (auth_type, auth_header_name) =
                    auth_parts(config.auth).ok_or(McpProbeError::invalid("validation_failed"))?;
                if config.api_key.is_some() && config.saved_credential.is_some()
                    || auth_type == "none"
                        && (config.api_key.is_some() || config.saved_credential.is_some())
                {
                    return Err(McpProbeError::invalid("credential_invalid"));
                }
                let credential = if let Some(saved) = config.saved_credential {
                    if saved.key != config.key {
                        return Err(McpProbeError::invalid("credential_invalid"));
                    }
                    let row = mcp_by(self.database, &saved.key)
                        .await
                        .map_err(McpProbeError::database)?;
                    if row.revision != saved.expected_revision {
                        return Err(McpProbeError {
                            status: http::StatusCode::CONFLICT,
                            code: "revision_conflict",
                        });
                    }
                    if row.auth_type != auth_type || row.auth_header_name != auth_header_name {
                        return Err(McpProbeError::invalid("credential_invalid"));
                    }
                    let reference = crate::database::credentials::reference(
                        &format!("mcp:{}", row.key),
                        row.credential_json.as_deref(),
                        crate::database::secrets::mcp_secret_env(&row.key, &row.auth_type),
                    )
                    .ok_or(McpProbeError::invalid("credential_missing"))?;
                    let reference = crate::database::secrets::SecretRef::parse(reference)
                        .map_err(|_| McpProbeError::invalid("credential_invalid"))?;
                    Some(
                        self.secrets
                            .resolve(&reference)
                            .map_err(|_| McpProbeError::invalid("credential_invalid"))?,
                    )
                } else {
                    config.api_key
                };
                if credential
                    .as_ref()
                    .is_some_and(|v| !crate::database::credentials::valid_input(v))
                {
                    return Err(McpProbeError::invalid("credential_invalid"));
                }
                let has_credential = credential.is_some();
                let secrets: Arc<dyn SecretResolver> = match credential {
                    Some(value) => Arc::new(InlineCredential(value)),
                    None => self.secrets.clone(),
                };
                (
                    AdmittedMcpServer {
                        id: 0,
                        revision: 1,
                        key: config.key,
                        url: config.url,
                        headers_json: "{}".into(),
                        auth_type,
                        auth_header_name,
                        secret_ref: has_credential.then(|| "DRAFT".into()),
                        connect_timeout_ms: config.connect_timeout_ms as i64,
                        request_timeout_ms: config.request_timeout_ms as i64,
                    },
                    secrets,
                    "draft",
                )
            }
        };
        let manager = self.manager.clone();
        // The bounded attempt owns admission until RMCP acknowledges close, even if HTTP leaves.
        let attempt = tokio::spawn(async move {
            let _permit = permit;
            if !manager.gate.is_open() {
                return Err(McpProbeError {
                    status: http::StatusCode::SERVICE_UNAVAILABLE,
                    code: "server_is_shutting_down",
                });
            }
            manager
                .probe_catalog(&server, secrets.as_ref(), discover)
                .await
                .map_err(|reason| McpProbeError {
                    status: if matches!(reason, super::ExternalMcpExclusionReason::ToolsListTimeout)
                    {
                        http::StatusCode::GATEWAY_TIMEOUT
                    } else {
                        http::StatusCode::BAD_GATEWAY
                    },
                    code: reason.as_str(),
                })
        });
        let catalog = attempt.await.map_err(|_| McpProbeError {
            status: http::StatusCode::SERVICE_UNAVAILABLE,
            code: "mcp_unavailable",
        })??;
        let mut result = serde_json::json!({"test_source":source,"status":"success","elapsed_ms":started.elapsed().as_millis(),"connected_at_test_time":true});
        if let Some(catalog) = catalog {
            result["complete"] = serde_json::json!(true);
            result["dropped_tools"] = serde_json::json!(catalog.dropped_tools);
            result["tools"] = serde_json::json!(catalog.tools.into_iter().map(|tool| serde_json::json!({"original_name":tool.original_name,"llm_name":tool.llm_name,"description":tool.description,"input_schema":tool.input_schema})).collect::<Vec<_>>());
        }
        Ok(result)
    }
}
