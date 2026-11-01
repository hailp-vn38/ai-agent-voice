use serde::Deserialize;
use std::path::PathBuf;

/// Deployment configuration for Memory Core (`[memory]`). Off by default.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MemorySectionConfig {
    pub enabled: bool,
    pub vault_dir: PathBuf,
    pub voice_summary_max_bytes: usize,
    pub max_facts_per_scope: usize,
    pub max_fact_body_chars: usize,
}

impl Default for MemorySectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            vault_dir: PathBuf::from("data/memory-vault"),
            voice_summary_max_bytes: 4_000,
            max_facts_per_scope: 500,
            max_fact_body_chars: 2_000,
        }
    }
}

impl MemorySectionConfig {
    /// Limits are checked even while disabled, so enabling Memory later cannot surface a bad value.
    pub fn valid(&self) -> bool {
        !self.vault_dir.as_os_str().is_empty()
            && self.voice_summary_max_bytes > 0
            && self.max_facts_per_scope > 0
            && self.max_fact_body_chars > 0
    }
}
