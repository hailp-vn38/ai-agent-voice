//! Immutable qualification scenario identities and handoff state.

use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs::OpenOptions, io::Write, path::Path};
use thiserror::Error;

const RUN_ID_PREFIX: &str = "it_";
const RUN_TOKEN_LEN: usize = 24;

#[derive(Debug, Error)]
pub enum ScenarioError {
    #[error("scenario_identity_invalid")]
    Identity,
    #[error("scenario_state_exists")]
    StateExists,
    #[error("scenario_state_invalid")]
    StateInvalid,
    #[error("scenario_state_io")]
    Io,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioPlan {
    pub run_id: String,
    pub spec_sha256: String,
    pub resource_keys: BTreeMap<String, String>,
    pub device_id: String,
}

impl ScenarioPlan {
    pub fn materialize(
        raw_spec: &[u8],
        run_id: &str,
        key_prefix: &str,
        roles: &[&str],
    ) -> Result<Self, ScenarioError> {
        let token = run_id
            .strip_prefix(RUN_ID_PREFIX)
            .ok_or(ScenarioError::Identity)?;
        if token.len() != RUN_TOKEN_LEN
            || !token
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || key_prefix.is_empty()
        {
            return Err(ScenarioError::Identity);
        }
        let mut resource_keys = BTreeMap::new();
        for role in roles {
            let key = format!("{key_prefix}_{token}_{role}");
            if key.len() > 64
                || !key.bytes().enumerate().all(|(index, byte)| {
                    (index == 0 && byte.is_ascii_lowercase())
                        || byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || byte == b'_'
                })
            {
                return Err(ScenarioError::Identity);
            }
            resource_keys.insert((*role).to_owned(), key);
        }
        let device_id = format!("{key_prefix}_{token}_device");
        if device_id.len() > 128 {
            return Err(ScenarioError::Identity);
        }
        Ok(Self {
            run_id: run_id.into(),
            spec_sha256: sha256(raw_spec),
            resource_keys,
            device_id,
        })
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ScenarioState {
    pub schema_version: u8,
    pub run_id: String,
    pub scenario_spec_sha256: String,
    pub resource_keys: BTreeMap<String, String>,
    pub device_id: String,
}

impl ScenarioState {
    pub fn from_plan(plan: &ScenarioPlan) -> Self {
        Self {
            schema_version: 1,
            run_id: plan.run_id.clone(),
            scenario_spec_sha256: plan.spec_sha256.clone(),
            resource_keys: plan.resource_keys.clone(),
            device_id: plan.device_id.clone(),
        }
    }
    pub fn validate_spec(&self, raw_spec: &[u8]) -> Result<(), ScenarioError> {
        if self.schema_version != 1 || self.scenario_spec_sha256 != sha256(raw_spec) {
            return Err(ScenarioError::StateInvalid);
        }
        Ok(())
    }
    pub fn write_create_new(&self, path: &Path) -> Result<(), ScenarioError> {
        if path.exists() {
            return Err(ScenarioError::StateExists);
        }
        let parent = path
            .parent()
            .filter(|parent| parent.is_dir())
            .ok_or(ScenarioError::Io)?;
        let temp = parent.join(format!(
            ".{}.tmp",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("state")
        ));
        let result = (|| -> Result<(), ScenarioError> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)
                .map_err(|_| ScenarioError::Io)?;
            serde_json::to_writer(&mut file, self).map_err(|_| ScenarioError::Io)?;
            file.write_all(b"\n").map_err(|_| ScenarioError::Io)?;
            file.sync_all().map_err(|_| ScenarioError::Io)?;
            std::fs::hard_link(&temp, path).map_err(|_| ScenarioError::StateExists)?;
            std::fs::remove_file(&temp).map_err(|_| ScenarioError::Io)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result
    }
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_is_only_written_after_a_complete_immutable_plan() {
        let raw = b"[scenario]\nkey_prefix = 'qual'\n";
        let plan = ScenarioPlan::materialize(
            raw,
            "it_0123456789abcdef01234567",
            "qual",
            &["agent", "template", "asr"],
        )
        .unwrap();
        assert_eq!(
            plan.resource_keys["agent"],
            "qual_0123456789abcdef01234567_agent"
        );
        let state = ScenarioState::from_plan(&plan);
        state.validate_spec(raw).unwrap();
        assert!(state.validate_spec(b"changed").is_err());
    }
}
