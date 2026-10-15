//! Settings live apart from installation status and Core activation.
use super::{PluginError, PluginStore};
use crate::database::{
    credentials::CredentialCipher, plugin_configs::ConfigWriteError, secrets::SecretValue,
};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

mod secrets;
mod validation;

pub(crate) struct ValueSettingsContract {
    pub schema: Value,
    pub schema_digest: String,
    pub package_digest: Option<String>,
}

impl PluginStore {
    /// The catalog key, not the frozen plugin_id, owns settings.
    pub(crate) async fn value_settings_contract(
        &self,
        key: &str,
    ) -> Result<ValueSettingsContract, PluginError> {
        let (schema, package_digest) = if crate::tools::core::known_group(key) {
            (
                json!({"type":"object","properties":{},"additionalProperties":false}),
                None,
            )
        } else {
            let row = self.row(key).await?;
            let manifest = row.manifest()?;
            let schema = if let Some(settings) = manifest.settings {
                let source = self
                    .root
                    .join("packages")
                    .join(key)
                    .join(&row.version)
                    .join(&row.package_digest)
                    .join("package.zip");
                let bytes = tokio::fs::read(source)
                    .await
                    .map_err(|_| PluginError::StorageUnavailable)?;
                let package = super::archive::validate(&bytes)?;
                if package.digest != row.package_digest {
                    return Err(PluginError::InvalidPackage);
                }
                let data = package
                    .files
                    .iter()
                    .find(|(name, _)| name == &settings.schema)
                    .map(|(_, data)| data)
                    .ok_or(PluginError::InvalidPackage)?;
                serde_json::from_slice::<Value>(data).map_err(|_| PluginError::InvalidPackage)?
            } else {
                json!({"type":"object","properties":{},"additionalProperties":false})
            };
            (schema, Some(row.package_digest))
        };
        let encoded = serde_json::to_vec(&schema).map_err(|_| PluginError::InvalidPackage)?;
        let schema_digest = Sha256::digest(&encoded)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(ValueSettingsContract {
            schema,
            schema_digest,
            package_digest,
        })
    }

    /// A package with required settings cannot enter the ready state unconfigured.
    /// Full schema validation already happened at the Settings write seam.
    pub(crate) async fn value_settings_activatable(&self, key: &str) -> Result<(), PluginError> {
        let contract = self.value_settings_contract(key).await?;
        let Some(current) = self.database.plugin_config_version(key).await? else {
            let required = contract.schema["required"].as_array();
            return if required.is_none_or(|fields| fields.is_empty()) {
                Ok(())
            } else {
                Err(PluginError::PermissionRequired)
            };
        };
        if current.schema_digest != contract.schema_digest
            || current.package_digest != contract.package_digest
        {
            return Err(PluginError::PermissionRequired);
        }
        let public: Map<String, Value> = serde_json::from_str(&current.public_json)
            .map_err(|_| PluginError::DatabaseUnavailable)?;
        let secrets: Map<String, Value> = serde_json::from_str(&current.secrets_json)
            .map_err(|_| PluginError::DatabaseUnavailable)?;
        if contract.schema["required"]
            .as_array()
            .is_some_and(|fields| {
                fields.iter().any(|field| {
                    field.as_str().is_none_or(|name| {
                        !public.contains_key(name) && !secrets.contains_key(name)
                    })
                })
            })
        {
            return Err(PluginError::PermissionRequired);
        }
        Ok(())
    }

    /// Resolve immutable Job-owned values. No current config fallback and no
    /// externally supplied credential references are accepted.
    pub(crate) async fn resolved_job_settings(
        &self,
        job_id: &str,
        choice: &super::FrozenChoice,
    ) -> Result<Option<Value>, PluginError> {
        let key = if crate::tools::core::is_core(choice) {
            crate::tools::core::group_for(&choice.operation)
                .ok_or(PluginError::PermissionRequired)?
        } else {
            choice.plugin_id.as_str()
        };
        let contract = self.value_settings_contract(key).await?;
        let declared = if crate::tools::core::is_core(choice) {
            false
        } else {
            self.row(key).await?.manifest()?.settings.is_some()
        };
        let Some(pinned) = self.database.pinned_plugin_config(job_id, key).await? else {
            // Legacy Jobs have no value-settings authority, even when a newer
            // Admin configuration now exists. An opted-in worker receives {}.
            return Ok(declared.then(|| json!({})));
        };
        if pinned.schema_digest != contract.schema_digest
            || pinned.package_digest != contract.package_digest
            || (!crate::tools::core::is_core(choice)
                && pinned.package_digest.as_deref() != Some(choice.package_digest.as_str()))
        {
            return Err(PluginError::PermissionRequired);
        }
        let mut values: Map<String, Value> = serde_json::from_str(&pinned.public_json)
            .map_err(|_| PluginError::DatabaseUnavailable)?;
        let secrets: Map<String, Value> = serde_json::from_str(&pinned.secrets_json)
            .map_err(|_| PluginError::DatabaseUnavailable)?;
        if !secrets.is_empty() {
            let cipher = CredentialCipher::from_environment()
                .map_err(|_| PluginError::StorageUnavailable)?;
            for (name, encoded) in secrets {
                let encrypted = encoded.as_str().ok_or(PluginError::PermissionRequired)?;
                let scope = format!(
                    "plugin-value:{key}:{}:{}:{}:{name}",
                    pinned.revision,
                    pinned.schema_digest,
                    pinned.package_digest.as_deref().unwrap_or("core")
                );
                if contract.schema["properties"][name.as_str()]["x-secret"] != true {
                    return Err(PluginError::PermissionRequired);
                }
                let secret = secrets::open_ref(&cipher, encrypted, &scope)?;
                values.insert(name, Value::String(secret.expose().to_owned()));
            }
        }
        let value = Value::Object(values);
        let validator =
            jsonschema::validator_for(&contract.schema).map_err(|_| PluginError::InvalidPackage)?;
        if !validator.is_valid(&value) {
            return Err(PluginError::PermissionRequired);
        }
        validation::validate_worker_budget(&value)?;
        Ok(declared.then_some(value))
    }

    pub(crate) async fn operation_descriptor(
        &self,
        key: &str,
        operation: &str,
    ) -> Result<Value, PluginError> {
        let choice = if crate::tools::core::known_group(key) {
            if crate::tools::core::group_for(operation) != Some(key) {
                return Err(PluginError::PermissionRequired);
            }
            super::FrozenChoice {
                plugin_id: "core".into(),
                operation: operation.into(),
                version: "1".into(),
                package_digest: String::new(),
                installation_digest: String::new(),
            }
        } else {
            let row = self.row(key).await?;
            if !row.manifest()?.operations.contains_key(operation) {
                return Err(PluginError::NotFound);
            }
            super::FrozenChoice {
                plugin_id: key.into(),
                operation: operation.into(),
                version: row.version,
                package_digest: row.package_digest,
                installation_digest: row.installation_digest,
            }
        };
        self.operation_contract(&choice).await
    }

    pub(crate) async fn value_settings_view(&self, key: &str) -> Result<Value, PluginError> {
        let contract = self.value_settings_contract(key).await?;
        let current = self.database.plugin_config_version(key).await?;
        let (revision, values, secret_set, needs_reconfiguration) = if let Some(row) = current {
            let changed = row.schema_digest != contract.schema_digest
                || row.package_digest != contract.package_digest;
            if changed {
                // The prior revision stays immutable and pinned by older Jobs,
                // but does not donate credentials to a new plugin package.
                (row.revision, json!({}), json!({}), true)
            } else {
                let secrets: Value = serde_json::from_str(&row.secrets_json)
                    .map_err(|_| PluginError::DatabaseUnavailable)?;
                let public: Value = serde_json::from_str(&row.public_json)
                    .map_err(|_| PluginError::DatabaseUnavailable)?;
                let set: Map<String, Value> = secrets
                    .as_object()
                    .ok_or(PluginError::DatabaseUnavailable)?
                    .keys()
                    .map(|name| (name.clone(), Value::Bool(true)))
                    .collect();
                (row.revision, public, Value::Object(set), false)
            }
        } else {
            (0, json!({}), json!({}), false)
        };
        Ok(json!({"plugin_key":key,"revision":revision,"values":values,
            "secret_set":secret_set,"needs_reconfiguration":needs_reconfiguration,
            "schema":contract.schema,
            "schema_digest":contract.schema_digest,"package_digest":contract.package_digest}))
    }

    /// Returns the new settings revision; secrets never enter the read model.
    pub(crate) async fn save_value_settings(
        &self,
        key: &str,
        expected: i64,
        values: Value,
    ) -> Result<Value, PluginError> {
        let contract = self.value_settings_contract(key).await?;
        let submitted = values.as_object().ok_or(PluginError::PermissionRequired)?;
        let properties = contract.schema["properties"]
            .as_object()
            .ok_or(PluginError::InvalidPackage)?;
        let old = self.database.plugin_config_version(key).await?;
        if old.as_ref().map_or(0, |row| row.revision) != expected {
            return Err(PluginError::RevisionConflict);
        }
        let compatible_previous = old.as_ref().filter(|previous| {
            previous.schema_digest == contract.schema_digest
                && previous.package_digest == contract.package_digest
        });
        let mut public = Map::new();
        let mut sealed = compatible_previous
            .map(|row| serde_json::from_str::<Map<String, Value>>(&row.secrets_json))
            .transpose()
            .map_err(|_| PluginError::DatabaseUnavailable)?
            .unwrap_or_default();
        let mut checked = Map::new();
        let mut cipher: Option<CredentialCipher> = None;
        for (name, field) in properties {
            if field["x-secret"] == true {
                let raw = submitted.get(name);
                match raw {
                    Some(Value::String(secret)) => {
                        if cipher.is_none() {
                            cipher = Some(
                                CredentialCipher::from_environment()
                                    .map_err(|_| PluginError::StorageUnavailable)?,
                            );
                        }
                        let scope = format!(
                            "plugin-value:{key}:{}:{}:{}:{name}",
                            expected.saturating_add(1),
                            contract.schema_digest,
                            contract.package_digest.as_deref().unwrap_or("core")
                        );
                        let sealed_value = secrets::seal_ref(
                            cipher.as_ref().unwrap(),
                            &SecretValue::new(secret.clone()),
                            &scope,
                        )?;
                        sealed.insert(name.clone(), Value::String(sealed_value));
                        checked.insert(name.clone(), Value::String(secret.clone()));
                    }
                    Some(Value::Null) => {
                        sealed.remove(name);
                    }
                    Some(_) => return Err(PluginError::PermissionRequired),
                    None => {
                        if let Some(existing) = sealed.get(name).and_then(Value::as_str) {
                            if cipher.is_none() {
                                cipher = Some(
                                    CredentialCipher::from_environment()
                                        .map_err(|_| PluginError::StorageUnavailable)?,
                                );
                            }
                            // Verify ownership of the old sealed reference rather than
                            // accepting a ciphertext copied from another plugin or field.
                            let previous = old.as_ref().ok_or(PluginError::PermissionRequired)?;
                            let old_scope = format!(
                                "plugin-value:{key}:{}:{}:{}:{name}",
                                previous.revision,
                                previous.schema_digest,
                                previous.package_digest.as_deref().unwrap_or("core")
                            );
                            let value =
                                secrets::open_ref(cipher.as_ref().unwrap(), existing, &old_scope)?;
                            let new_scope = format!(
                                "plugin-value:{key}:{}:{}:{}:{name}",
                                expected.saturating_add(1),
                                contract.schema_digest,
                                contract.package_digest.as_deref().unwrap_or("core")
                            );
                            let rotated =
                                secrets::seal_ref(cipher.as_ref().unwrap(), &value, &new_scope)?;
                            sealed.insert(name.clone(), Value::String(rotated));
                            checked.insert(name.clone(), Value::String(value.expose().to_owned()));
                        }
                    }
                }
            } else if let Some(value) = submitted.get(name) {
                public.insert(name.clone(), value.clone());
                checked.insert(name.clone(), value.clone());
            }
        }
        if submitted.keys().any(|name| !properties.contains_key(name)) {
            return Err(PluginError::PermissionRequired);
        }
        let validator =
            jsonschema::validator_for(&contract.schema).map_err(|_| PluginError::InvalidPackage)?;
        let resolved = Value::Object(checked);
        // JSONL requests must remain below 65,535 bytes even when arguments
        // and protocol envelope are added. Bound the entire decrypted Settings
        // snapshot (including names, escaping, and secret values) to 16 KiB.
        if !validator.is_valid(&resolved) {
            return Err(PluginError::PermissionRequired);
        }
        validation::validate_worker_budget(&resolved)?;
        let public_values = Value::Object(public);
        let secret_set: Map<String, Value> = sealed
            .keys()
            .map(|name| (name.clone(), Value::Bool(true)))
            .collect();
        let revision = self
            .database
            .write_plugin_config(
                key,
                expected,
                &contract.schema_digest,
                contract.package_digest.as_deref(),
                &public_values,
                &Value::Object(sealed),
            )
            .await
            .map_err(|e| match e {
                ConfigWriteError::Conflict => PluginError::RevisionConflict,
                ConfigWriteError::Storage => PluginError::DatabaseUnavailable,
            })?;
        // Build the response from the CAS commit; no second read can turn a
        // successful write into an apparent failure. Never return sealed values.
        Ok(
            json!({"plugin_key":key,"revision":revision,"values":public_values,
            "secret_set":secret_set,"needs_reconfiguration":false,
            "schema":contract.schema,
            "schema_digest":contract.schema_digest,"package_digest":contract.package_digest}),
        )
    }
}
