//! Startup partition of database providers into required, optional and unbound sets.
//!
//! The plan is derived from the persisted Device → Agent → Template → Provider graph plus the
//! deployment's server provider defaults.  It never consults `AppConfig::effective_agent`,
//! because a per-deployment agent override is not evidence that a database provider is needed.

use std::collections::BTreeSet;

use crate::config::ProviderDefaultsConfig;

use super::{Database, DatabaseError};

/// How startup must treat one database provider instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderLoadRequirement {
    /// Referenced by a server provider default or an enabled Agent default Template.  A
    /// configuration, secret or runtime failure blocks startup before the listener binds.
    Required,
    /// Referenced only by an enabled non-default Template assignment.  A failure leaves the
    /// runtime unavailable and excludes that candidate without blocking boot.
    Optional,
    /// Not referenced by any enabled assignment.  Configuration is still validated so Admin
    /// inspection is honest, but no secret is resolved and no runtime is built.
    Unbound,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderLoadPlan {
    required: BTreeSet<String>,
    optional: BTreeSet<String>,
}

impl ProviderLoadPlan {
    /// Builds a plan from two provider-key sets.  A key reachable through both is loaded once,
    /// as required, so one provider is never materialized twice.
    pub fn new(
        required: impl IntoIterator<Item = String>,
        optional: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut plan = Self {
            required: required.into_iter().collect(),
            optional: optional.into_iter().collect(),
        };
        plan.optional.retain(|key| !plan.required.contains(key));
        plan
    }

    /// Plan for a deployment whose database holds no provider rows: only the server provider
    /// defaults are required, and they are already materialized by the local loader.
    pub fn from_server_defaults(defaults: &ProviderDefaultsConfig) -> Self {
        Self::new(
            [
                defaults.vad.as_str(),
                defaults.asr.as_str(),
                defaults.llm.as_str(),
                defaults.tts.as_str(),
            ]
            .into_iter()
            .filter(|key| !key.is_empty())
            .map(str::to_owned),
            [],
        )
    }

    pub fn requirement(&self, key: &str) -> ProviderLoadRequirement {
        if self.required.contains(key) {
            ProviderLoadRequirement::Required
        } else if self.optional.contains(key) {
            ProviderLoadRequirement::Optional
        } else {
            ProviderLoadRequirement::Unbound
        }
    }

    pub fn is_required(&self, key: &str) -> bool {
        self.requirement(key) == ProviderLoadRequirement::Required
    }

    pub fn required_keys(&self) -> &BTreeSet<String> {
        &self.required
    }

    pub fn optional_keys(&self) -> &BTreeSet<String> {
        &self.optional
    }
}

impl Database {
    /// Reads the persisted graph once, before any runtime is constructed.  Two unions are enough:
    /// a provider referenced by an enabled Agent default Template is required, and one reachable
    /// only through enabled non-default assignments is an optional switch candidate.
    pub async fn provider_load_plan(
        &self,
        defaults: &ProviderDefaultsConfig,
    ) -> Result<ProviderLoadPlan, DatabaseError> {
        let referenced = sqlx::query_as::<_, (String, i64)>(
            "SELECT DISTINCT p.key, ata.is_default \
             FROM template_provider_bindings b \
             JOIN providers p ON p.id = b.provider_id \
             JOIN agent_templates t ON t.id = b.template_id \
             JOIN agent_template_assignments ata ON ata.template_id = t.id \
             JOIN agents a ON a.id = ata.agent_id \
             WHERE p.enabled = 1 AND t.enabled = 1 AND ata.enabled = 1 AND a.enabled = 1",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(super::map_sqlx_error)?;

        let plan = ProviderLoadPlan::from_server_defaults(defaults);
        let mut required = plan
            .required_keys()
            .iter()
            .cloned()
            .collect::<Vec<String>>();
        let mut optional = Vec::new();
        for (key, is_default) in referenced {
            if is_default == 1 {
                required.push(key);
            } else {
                optional.push(key);
            }
        }
        Ok(ProviderLoadPlan::new(required, optional))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> ProviderDefaultsConfig {
        ProviderDefaultsConfig {
            vad: "silero_default".into(),
            asr: "gipformer_vi".into(),
            llm: "openai_primary".into(),
            tts: "zerotts_maichi".into(),
            vision: None,
        }
    }

    #[test]
    fn server_defaults_are_required_and_never_optional() {
        let plan = ProviderLoadPlan::from_server_defaults(&defaults());
        assert_eq!(
            plan.requirement("silero_default"),
            ProviderLoadRequirement::Required
        );
        assert!(plan.optional_keys().is_empty());
        assert_eq!(
            plan.requirement("unbound"),
            ProviderLoadRequirement::Unbound
        );
    }
}
