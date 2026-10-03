//! Client-owned wire DTOs. They intentionally do not reuse server handler/domain types.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
pub struct CreateAgentRequest {
    pub key: String,
    pub name: String,
    pub description: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct AgentView {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub enabled: bool,
    pub revision: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct CreateTemplateRequest {
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub language: String,
    pub prompt: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct TemplateView {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub enabled: bool,
    pub revision: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct CreateProviderRequest {
    pub key: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub adapter: String,
    pub config_json: serde_json::Value,
    pub secret_ref: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct ProviderView {
    pub id: i64,
    pub key: String,
    pub adapter: String,
    pub revision: u64,
    pub runtime_status: String,
    pub runtime_matches_desired: bool,
    pub requires_restart: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct BindTemplateProviderRequest {
    pub provider_key: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct CreateDeviceRequest {
    pub device_id: String,
    pub agent_key: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub metadata_json: Option<serde_json::Value>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct DeviceView {
    pub id: i64,
    pub device_id: String,
    pub agent_key: String,
    pub enabled: bool,
    pub revision: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct CreateMcpServerRequest {
    pub key: String,
    pub name: String,
    pub url: String,
    pub headers: serde_json::Map<String, serde_json::Value>,
    pub auth: McpAuth,
    pub connect_timeout_ms: u64,
    pub request_timeout_ms: u64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpAuth {
    None,
}
#[derive(Clone, Debug, Deserialize)]
pub struct McpServerView {
    pub key: String,
    pub revision: u64,
    pub enabled: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct McpBindingRequest {
    pub enabled: bool,
    pub required: bool,
}
