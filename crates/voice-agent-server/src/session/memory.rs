//! What a Voice Session holds of Memory: the vault and mode admission fixed for it.

use crate::{
    database::{Database, devices::memory_binding},
    services::memory::{MemoryContextHandle, MemoryCore, MemoryMode, MemoryScope},
};
use std::sync::Arc;

/// Built only from the session's own `device_db_id`. A Studio Assistant session therefore reaches
/// the Virtual Device's vault, whatever Bound Reminder Target it holds.
#[derive(Clone)]
pub struct SessionMemory {
    pub core: Arc<MemoryCore>,
    pub scope: MemoryScope,
    pub mode: MemoryMode,
    pub device_db_id: i64,
    pub handle: MemoryContextHandle,
}

impl SessionMemory {
    /// `None` means no Memory for this session: disabled, no such Device, a Device on `off`, or a
    /// failed lookup. There is no error to return, because Memory never refuses a session.
    pub async fn admit(
        core: Option<&Arc<MemoryCore>>,
        database: Option<&Database>,
        device_db_id: i64,
    ) -> Option<Self> {
        let Some(core) = core else {
            return not_admitted(device_db_id, "memory_core_unavailable");
        };
        let Some(database) = database else {
            return not_admitted(device_db_id, "database_unavailable");
        };
        if device_db_id <= 0 {
            return not_admitted(device_db_id, "no_device_row");
        }
        let binding = match memory_binding(database, device_db_id).await {
            Ok(Some(binding)) => binding,
            Ok(None) => return not_admitted(device_db_id, "device_not_found"),
            Err(_) => {
                tracing::warn!(
                    subsystem = "voice.ws",
                    event = "memory_context_degraded",
                    reason = "binding_lookup_failed"
                );
                return None;
            }
        };
        let mode = binding.mode;
        if mode == MemoryMode::Off {
            return not_admitted(device_db_id, "mode_off");
        }
        let scope = MemoryScope::for_stored_device(&binding.device_id, &binding.kind);
        let handle = core.context(&scope).await;
        let context = handle.current();
        tracing::info!(
            subsystem = "voice.ws",
            event = "voice_memory_session_admitted",
            device_db_id,
            device_kind = %binding.kind,
            mode = ?mode,
            summary_rows = context.rows.len(),
            summary_hidden = context.hidden,
            "Voice Session admitted to Memory"
        );
        Some(Self {
            core: Arc::clone(core),
            scope,
            mode,
            device_db_id,
            handle,
        })
    }
}

fn not_admitted(device_db_id: i64, reason: &'static str) -> Option<SessionMemory> {
    tracing::info!(
        subsystem = "voice.ws",
        event = "voice_memory_session_not_admitted",
        device_db_id,
        reason,
        "Voice Session runs without Memory"
    );
    None
}
