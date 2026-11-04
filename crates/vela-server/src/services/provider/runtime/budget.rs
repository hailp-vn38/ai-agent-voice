use super::RuntimeError;
use serde::Deserialize;

/// Deployment estimates, never a hard OS RAM guarantee.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeLimits {
    pub max_parallel_loads: usize,
    pub max_pending_loads: usize,
    pub max_waiters: usize,
    pub max_resident_bytes: u64,
    pub max_resources: usize,
    pub max_version_entries: usize,
    pub admission_timeout_ms: u64,
    pub failure_cooldown_ms: u64,
    pub idle_ttl_ms: u64,
}

impl RuntimeLimits {
    pub(crate) fn validate(&self) -> Result<(), RuntimeError> {
        if !(1..=16).contains(&self.max_parallel_loads)
            || self.max_pending_loads > 256
            || !(1..=4096).contains(&self.max_waiters)
            || self.max_resident_bytes == 0
            || !(1..=256).contains(&self.max_resources)
            || !(self.max_resources..=4096).contains(&self.max_version_entries)
            || !(1..=120_000).contains(&self.admission_timeout_ms)
            || !(1..=60_000).contains(&self.failure_cooldown_ms)
            || !(1..=86_400_000).contains(&self.idle_ttl_ms)
        {
            return Err(RuntimeError::Configuration);
        }
        Ok(())
    }
}
