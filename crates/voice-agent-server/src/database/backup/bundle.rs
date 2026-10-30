use super::{BackupCompanionSettings, BackupScope};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct DatabaseBackupBundle {
    pub schema_version: u32,
    pub server_version: String,
    pub created_at: i64,
    pub scope: BackupScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum_sha256: Option<String>,
    #[serde(default)]
    pub providers: Vec<BackupProviderRow>,
    #[serde(default)]
    pub agent_templates: Vec<BackupTemplateRow>,
    #[serde(default)]
    pub template_provider_bindings: Vec<BackupTemplateProviderBindingRow>,
    #[serde(default)]
    pub agents: Vec<BackupAgentRow>,
    #[serde(default)]
    pub agent_template_assignments: Vec<BackupAgentTemplateAssignmentRow>,
    #[serde(default)]
    pub mcp_servers: Vec<BackupMcpServerRow>,
    #[serde(default)]
    pub agent_mcp_bindings: Vec<BackupAgentMcpBindingRow>,
    #[serde(default)]
    pub external_tool_observations: Vec<BackupExternalToolObservationRow>,
    #[serde(default)]
    pub agent_external_tool_allowlist: Vec<BackupAgentToolAllowlistRow>,
    #[serde(default)]
    pub devices: Vec<BackupDeviceRow>,
    #[serde(default)]
    pub speakers: Vec<BackupSpeakerRow>,
    #[serde(default)]
    pub speaker_voiceprints: Vec<BackupSpeakerVoiceprintRow>,
    #[serde(default)]
    pub agent_speaker_policies: Vec<BackupAgentSpeakerPolicyRow>,
    #[serde(default)]
    pub agent_speaker_candidates: Vec<BackupAgentSpeakerCandidateRow>,
    #[serde(default)]
    pub agent_speaker_template_grants: Vec<BackupAgentSpeakerTemplateGrantRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_calibration_catalog: Option<BackupSpeakerCalibrationCatalogRow>,
    #[serde(default)]
    pub speaker_calibration_evidence: Vec<BackupSpeakerCalibrationEvidenceRow>,
    #[serde(default)]
    pub llm_gateway_accounts: Vec<BackupLlmGatewayAccountRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_settings_state: Option<BackupNotificationSettingsRow>,
    #[serde(default)]
    pub notification_channel_configs: Vec<BackupNotificationChannelConfigRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_mcp_grants: Option<BackupSystemMcpGrantsRow>,
    #[serde(default)]
    pub device_reminders: Vec<BackupDeviceReminderRow>,
    #[serde(default)]
    pub history_messages: Vec<BackupHistoryMessageRow>,
    #[serde(default)]
    pub notifications_inbox: Vec<BackupNotificationInboxRow>,
    #[serde(default)]
    pub companion_settings: BackupCompanionSettings,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupProviderRow {
    pub key: String,
    pub name: String,
    pub provider_type: String,
    pub adapter: String,
    pub config_json: String,
    pub credential_json: Option<String>,
    pub enabled: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupTemplateRow {
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub language: String,
    pub prompt: String,
    pub enabled: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupTemplateProviderBindingRow {
    pub template_key: String,
    pub provider_type: String,
    pub provider_source: String,
    pub provider_key: Option<String>,
    pub config_provider_key: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupAgentRow {
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub enabled: i64,
    pub reminders_enabled: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupAgentTemplateAssignmentRow {
    pub agent_key: String,
    pub template_key: String,
    pub is_default: i64,
    pub enabled: i64,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupMcpServerRow {
    pub key: String,
    pub name: String,
    pub transport: String,
    pub url: String,
    pub headers_json: String,
    pub auth_type: String,
    pub auth_header_name: Option<String>,
    pub connect_timeout_ms: i64,
    pub request_timeout_ms: i64,
    pub enabled: i64,
    pub credential_json: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupAgentMcpBindingRow {
    pub agent_key: String,
    pub server_key: String,
    pub enabled: i64,
    pub required: i64,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupExternalToolObservationRow {
    pub server_key: String,
    pub original_name: String,
    pub description: String,
    pub input_schema: String,
    pub fingerprint: String,
    pub server_revision: i64,
    pub blocked: i64,
    pub revision: i64,
    pub observed_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupAgentToolAllowlistRow {
    pub agent_key: String,
    pub server_key: String,
    pub original_name: String,
    pub fingerprint: String,
    pub allowed: i64,
    pub sensitive: i64,
    pub revision: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupDeviceRow {
    pub device_id: String,
    pub kind: String,
    pub agent_key: String,
    pub template_key: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled: i64,
    pub metadata_json: Option<String>,
    pub last_seen_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupSpeakerRow {
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub enabled: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupSpeakerVoiceprintRow {
    pub speaker_key: String,
    pub embedding_space: String,
    pub provider_key: String,
    pub provider_revision: i64,
    pub dims: i64,
    pub vector_base64: String,
    pub revision: i64,
    pub sample_count: i64,
    pub browser_validation_status: String,
    pub enrolled_at: i64,
    pub updated_at: i64,
    pub calibration_revision: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupAgentSpeakerPolicyRow {
    pub agent_key: String,
    pub mode: String,
    pub revision: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupAgentSpeakerCandidateRow {
    pub agent_key: String,
    pub speaker_key: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupAgentSpeakerTemplateGrantRow {
    pub agent_key: String,
    pub speaker_key: String,
    pub template_key: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupSpeakerCalibrationCatalogRow {
    pub calibration_revision: String,
    pub source_digest: String,
    pub profiles_json: String,
    pub published_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupSpeakerCalibrationEvidenceRow {
    pub calibration_revision: String,
    pub candidate_set_digest: String,
    pub candidate_set_json: String,
    pub report_ref: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupLlmGatewayAccountRow {
    pub account_key: String,
    pub vendor: String,
    pub label: String,
    pub account_email: Option<String>,
    pub credential_json: String,
    pub expires_at: Option<i64>,
    pub enabled: i64,
    pub priority: i64,
    pub status: String,
    pub last_error: Option<String>,
    pub last_refreshed_at: Option<i64>,
    pub last_used_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupNotificationSettingsRow {
    pub web_ui_toast_enabled: i64,
    pub retention_days: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupNotificationChannelConfigRow {
    pub channel_kind: String,
    pub enabled: i64,
    pub target_json: String,
    pub subscribed_topics_json: String,
    pub encrypted_credential: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupSystemMcpGrantsRow {
    pub system_read: i64,
    pub mcp_read: i64,
    pub mcp_discover: i64,
    pub mcp_manage_request: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupDeviceReminderRow {
    pub id: String,
    pub device_id: String,
    #[serde(default = "default_physical_kind")]
    pub device_kind: String,
    pub message: String,
    pub due_at_ms: i64,
    pub timezone: String,
    pub status: String,
    pub row_revision: i64,
    pub schedule_revision: i64,
    pub source: String,
    pub attempts_used: i64,
    pub next_attempt_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub cancelled_at_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupHistoryMessageRow {
    pub session_id: String,
    pub sequence: i64,
    pub device_id: String,
    #[serde(default = "default_physical_kind")]
    pub device_kind: String,
    pub agent_key: String,
    pub template_key: Option<String>,
    pub turn_id: Option<String>,
    pub role: String,
    pub text: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BackupNotificationInboxRow {
    pub id: String,
    pub dedup_key: Option<String>,
    pub topic: String,
    pub severity: String,
    pub title: String,
    pub body: String,
    pub source_kind: Option<String>,
    pub source_id: Option<String>,
    pub metadata_json: String,
    pub is_read: i64,
    pub read_at_ms: Option<i64>,
    pub created_at_ms: i64,
}

fn default_physical_kind() -> String {
    "physical".to_owned()
}
