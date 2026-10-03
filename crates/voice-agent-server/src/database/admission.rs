//! Database read side of Voice Protocol Client admission.
//!
//! This module owns only the SQL that snapshots the Device → Agent → Template → Provider graph
//! before a WebSocket is upgraded.  Turning that snapshot into an Effective Session Profile is a
//! service responsibility and never re-queries SQLite.

use std::{collections::HashMap, sync::Arc};

use crate::config::DatabaseDevicesConfig;
use sqlx::{Sqlite, Transaction};
use thiserror::Error;

use super::{Database, DatabaseError, map_sqlx_error};

/// Immutable database facts captured before a WebSocket is upgraded.
///
/// The realtime actor keeps only what the Effective Session Profile carries; it never owns this
/// snapshot's repository and never re-reads a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceAdmissionGraph {
    pub device_db_id: i64,
    pub template_override_id: Option<i64>,
    pub agent: AdmittedAgent,
    /// Every assignment row the Agent has, including disabled ones.  A non-empty list means the
    /// Agent entered the Template mechanism and can never fall back to server defaults.
    pub assignments: Vec<AdmittedAssignment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedAgent {
    pub id: i64,
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedAssignment {
    pub template_id: i64,
    pub template_key: String,
    pub template_name: String,
    pub language: String,
    pub prompt: String,
    pub template_enabled: bool,
    pub template_revision: i64,
    pub is_default: bool,
    pub assignment_enabled: bool,
    pub bindings: Vec<AdmittedProviderBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedProviderBinding {
    pub provider_type: String,
    pub provider_key: String,
    pub provider_enabled: bool,
    /// Canonical credential-free config. None means invalid persisted config or a manually
    /// constructed legacy test graph; manager-backed admission refuses to acquire such a binding.
    pub snapshot: Option<Arc<super::DesiredProvider>>,
}

#[derive(Debug, Error)]
pub enum DeviceAdmissionError {
    #[error("device_not_admitted")]
    Denied,
    #[error("device_admission_unavailable")]
    Unavailable,
}

impl From<DatabaseError> for DeviceAdmissionError {
    fn from(_: DatabaseError) -> Self {
        Self::Unavailable
    }
}

impl Database {
    /// Resolves the Device and its enabled Agent in one pre-upgrade database operation.
    /// Unknown identities can be provisioned only through the explicit dev/migration switch.
    pub async fn admit_device(
        &self,
        device_id: &str,
        devices: &DatabaseDevicesConfig,
    ) -> Result<DeviceAdmissionGraph, DeviceAdmissionError> {
        match self.find_admission(device_id).await? {
            Some(admission) => Ok(admission),
            None if devices.auto_register => self.auto_register_and_admit(device_id, devices).await,
            None => Err(DeviceAdmissionError::Denied),
        }
    }

    async fn find_admission(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceAdmissionGraph>, DeviceAdmissionError> {
        let mut transaction = self.pool.begin().await.map_err(map_sqlx_error)?;
        let row = sqlx::query_as::<_, (i64, Option<i64>, i64, i64, i64, String)>(
            "SELECT d.id, d.template_id, a.id, d.enabled, a.enabled, a.key \
             FROM devices d JOIN agents a ON a.id = d.agent_id WHERE d.device_id = ?",
        )
        .bind(device_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(map_sqlx_error)?;
        let Some((
            device_db_id,
            template_override_id,
            agent_id,
            device_enabled,
            agent_enabled,
            key,
        )) = row
        else {
            return Ok(None);
        };

        if device_enabled == 0 || agent_enabled == 0 {
            return Err(DeviceAdmissionError::Denied);
        }
        let assignments = self.assignment_graph(agent_id, &mut transaction).await?;
        transaction.commit().await.map_err(map_sqlx_error)?;
        Ok(Some(DeviceAdmissionGraph {
            device_db_id,
            template_override_id,
            agent: AdmittedAgent { id: agent_id, key },
            assignments,
        }))
    }

    /// Bounded metadata/size checks and two graph reads cover the Template graph.  Bindings are read for every assigned
    /// Template in one statement so admission never scales with the number of assignments.
    async fn assignment_graph(
        &self,
        agent_id: i64,
        transaction: &mut Transaction<'_, Sqlite>,
    ) -> Result<Vec<AdmittedAssignment>, DeviceAdmissionError> {
        let assignment_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM (SELECT template_id FROM agent_template_assignments WHERE agent_id=? LIMIT 65)",
        ).bind(agent_id).fetch_one(&mut **transaction).await.map_err(map_sqlx_error)?;
        if assignment_count > 64 {
            return Err(DeviceAdmissionError::Unavailable);
        }
        // Check byte cost inside the same read transaction before copying potentially large
        // TEXT values. Counting duplicate bindings is conservative and keeps query allocation bounded.
        let bytes: i64 = sqlx::query_scalar(
            "SELECT COALESCE((SELECT SUM(length(CAST(t.key AS BLOB)) + length(CAST(t.name AS BLOB)) + \
                    length(CAST(t.language AS BLOB)) + length(CAST(t.prompt AS BLOB))) \
                FROM agent_template_assignments ata JOIN agent_templates t ON t.id=ata.template_id \
                WHERE ata.agent_id=?),0) + COALESCE((SELECT SUM(length(CAST(p.key AS BLOB)) + \
                    length(CAST(p.type AS BLOB)) + length(CAST(p.adapter AS BLOB)) + \
                    length(CAST(p.config_json AS BLOB)) + COALESCE(length(CAST(p.secret_ref AS BLOB)),0)) \
                FROM template_provider_bindings b JOIN providers p ON p.id=b.provider_id \
                WHERE b.template_id IN (SELECT template_id FROM agent_template_assignments WHERE agent_id=?)),0)",
        )
        .bind(agent_id)
        .bind(agent_id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(map_sqlx_error)?;
        if bytes > 2 * 1024 * 1024 {
            return Err(DeviceAdmissionError::Unavailable);
        }
        let rows = sqlx::query_as::<_, (i64, String, String, String, String, i64, i64, i64, i64)>(
            "SELECT t.id, t.key, t.name, t.language, t.prompt, t.enabled, t.revision, \
                    ata.is_default, ata.enabled \
             FROM agent_template_assignments ata \
             JOIN agent_templates t ON t.id = ata.template_id \
             WHERE ata.agent_id = ? ORDER BY t.id LIMIT 65",
        )
        .bind(agent_id)
        .fetch_all(&mut **transaction)
        .await
        .map_err(map_sqlx_error)?;
        if rows.len() > 64 {
            return Err(DeviceAdmissionError::Unavailable);
        }
        let bindings = sqlx::query_as::<_, (i64, String, i64, String, i64, String, String, String, Option<String>, i64)>(
            "SELECT b.template_id, b.provider_type, p.id, p.key, p.enabled, p.type, p.adapter, p.config_json, p.secret_ref, p.revision \
             FROM template_provider_bindings b \
             JOIN providers p ON p.id = b.provider_id \
             WHERE b.template_id IN (SELECT template_id FROM agent_template_assignments \
                                     WHERE agent_id = ?) \
             ORDER BY b.template_id, b.provider_type LIMIT 257",
        )
        .bind(agent_id)
        .fetch_all(&mut **transaction)
        .await
        .map_err(map_sqlx_error)?;

        if bindings.len() > 256 {
            return Err(DeviceAdmissionError::Unavailable);
        }
        let mut snapshot_bytes = rows
            .iter()
            .map(|row| row.1.len() + row.2.len() + row.3.len() + row.4.len())
            .sum::<usize>();
        let mut snapshots: HashMap<i64, Option<Arc<super::DesiredProvider>>> = HashMap::new();
        let mut assignments: Vec<AdmittedAssignment> = rows
            .into_iter()
            .map(
                |(
                    template_id,
                    template_key,
                    template_name,
                    language,
                    prompt,
                    template_enabled,
                    template_revision,
                    is_default,
                    assignment_enabled,
                )| AdmittedAssignment {
                    template_id,
                    template_key,
                    template_name,
                    language,
                    prompt,
                    template_enabled: template_enabled == 1,
                    template_revision,
                    is_default: is_default == 1,
                    assignment_enabled: assignment_enabled == 1,
                    bindings: Vec::new(),
                },
            )
            .collect();
        for (
            template_id,
            provider_type,
            id,
            provider_key,
            provider_enabled,
            kind,
            adapter,
            config_json,
            secret_ref,
            revision,
        ) in bindings
        {
            let snapshot = match snapshots.entry(id) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.get().clone(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    snapshot_bytes = snapshot_bytes.saturating_add(
                        provider_key.len()
                            + kind.len()
                            + adapter.len()
                            + config_json.len()
                            + secret_ref.as_ref().map_or(0, String::len),
                    );
                    if snapshot_bytes > 2 * 1024 * 1024 {
                        return Err(DeviceAdmissionError::Unavailable);
                    }
                    let validated =
                        super::provider_config::validate_raw(&adapter, &config_json).ok();
                    let valid_secret = secret_ref.as_ref().is_none_or(|reference| {
                        super::secrets::SecretRef::parse(reference.clone()).is_ok()
                    });
                    entry
                        .insert(validated.filter(|_| valid_secret).map(|config_json| {
                            Arc::new(super::DesiredProvider {
                                id,
                                key: provider_key.clone(),
                                kind,
                                adapter,
                                config_json,
                                secret_ref,
                                revision,
                            })
                        }))
                        .clone()
                }
            };
            let Some(assignment) = assignments
                .iter_mut()
                .find(|assignment| assignment.template_id == template_id)
            else {
                continue;
            };
            assignment.bindings.push(AdmittedProviderBinding {
                provider_type,
                provider_key,
                provider_enabled: provider_enabled == 1,
                snapshot,
            });
        }
        Ok(assignments)
    }

    async fn auto_register_and_admit(
        &self,
        device_id: &str,
        devices: &DatabaseDevicesConfig,
    ) -> Result<DeviceAdmissionGraph, DeviceAdmissionError> {
        let registered_at = super::unix_seconds().ok_or(DeviceAdmissionError::Unavailable)?;
        let metadata = serde_json::json!({
            "source": "auto_register",
            "registered_at": registered_at,
        })
        .to_string();
        sqlx::query(
            "INSERT OR IGNORE INTO devices \
             (device_id, agent_id, enabled, metadata_json, created_at, updated_at) \
             SELECT ?, id, 1, ?, ?, ? FROM agents WHERE key = ? AND enabled = 1",
        )
        .bind(device_id)
        .bind(metadata)
        .bind(registered_at)
        .bind(registered_at)
        .bind(&devices.auto_register_agent_key)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        // A racing connection may have inserted first. Re-read and enforce the same policy
        // rather than assuming the winner's binding is safe for this connection.
        self.find_admission(device_id)
            .await?
            .ok_or(DeviceAdmissionError::Denied)
    }
}
