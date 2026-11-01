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
        let (core, database) = (core?, database?);
        if device_db_id <= 0 {
            return None;
        }
        let binding = match memory_binding(database, device_db_id).await {
            Ok(binding) => binding?,
            Err(_) => {
                tracing::warn!(
                    event = "memory_context_degraded",
                    reason = "binding_lookup_failed"
                );
                return None;
            }
        };
        let mode = binding.mode;
        if mode == MemoryMode::Off {
            return None;
        }
        let scope = MemoryScope::for_stored_device(&binding.device_id, &binding.kind);
        let handle = core.context(&scope).await;
        Some(Self {
            core: Arc::clone(core),
            scope,
            mode,
            device_db_id,
            handle,
        })
    }
}
