use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::capabilities::{LanguageOption, ModelOption, VoiceOption};

#[derive(Serialize)]
pub struct DiscoveredCapabilities {
    pub models: &'static [ModelOption],
    pub voices: &'static [VoiceOption],
    pub languages: &'static [LanguageOption],
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderInspectError {
    #[error("invalid bootstrap selection")]
    InvalidSelection,
    #[error("bootstrap discovery is not supported")]
    Unsupported,
}

pub trait BootstrapCapabilityInspector: Send + Sync {
    fn inspect(&self, selection: &Value) -> Result<DiscoveredCapabilities, ProviderInspectError>;
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelSelection {
    #[serde(default)]
    pub(crate) model: Option<String>,
}

pub(crate) fn validate_model_selection(
    selection: &Value,
    model: &str,
) -> Result<(), ProviderInspectError> {
    let selection: ModelSelection = serde_json::from_value(selection.clone())
        .map_err(|_| ProviderInspectError::InvalidSelection)?;
    if selection
        .model
        .is_some_and(|selected| selected.len() > 128 || selected != model)
    {
        return Err(ProviderInspectError::InvalidSelection);
    }
    Ok(())
}
