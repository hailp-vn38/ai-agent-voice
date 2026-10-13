use super::{ProviderRuntimeManager, RuntimeError};
use std::sync::Arc;
use tokio::time::{Instant, timeout_at};

impl ProviderRuntimeManager {
    /// Registry synchronization blocks acquisition before native unload begins.
    /// Timeout stops only this waiter; the owner retains accounting until actual completion.
    pub async fn evict_idle(self: &Arc<Self>) -> Result<usize, RuntimeError> {
        self.drain_idle_until(self.admission_deadline(), false, None)
            .await
    }

    pub async fn evict_expired(self: &Arc<Self>) -> Result<usize, RuntimeError> {
        self.drain_idle_until(self.admission_deadline(), true, None)
            .await
    }

    async fn drain_idle_until(
        self: &Arc<Self>,
        deadline: Instant,
        expired_only: bool,
        max_drains: Option<usize>,
    ) -> Result<usize, RuntimeError> {
        let mut draining = Vec::new();
        for drain in self.begin_drains(expired_only, max_drains) {
            let manager = Arc::clone(self);
            let version = drain.version.clone();
            let generation = drain.generation;
            let (complete, task) = tokio::sync::oneshot::channel();
            let runtime = tokio::runtime::Handle::current();
            let spawn = std::thread::Builder::new()
                .name("provider-unload".into())
                .spawn(move || {
                    let _context = runtime.enter();
                    let _timer = manager.metrics().timer(super::RuntimePhase::Unload);
                    let acknowledged =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            drain.resource.unload()
                        }))
                        .unwrap_or(false);
                    manager.finish_drain(&version, generation, acknowledged);
                    let _ = complete.send(acknowledged);
                });
            if spawn.is_err() {
                self.finish_drain(&drain.version, generation, false);
            }
            draining.push((drain.version, generation, task));
        }
        let mut acknowledged = 0;
        for (version, generation, task) in draining {
            match timeout_at(deadline, task).await {
                Ok(Ok(true)) => acknowledged += 1,
                Ok(_) => return Err(RuntimeError::Quarantined),
                Err(_) => {
                    self.drain_timeout(&version, generation);
                    return Err(RuntimeError::Timeout);
                }
            }
        }
        Ok(acknowledged)
    }

    pub(super) async fn evict_pressure_until(
        self: &Arc<Self>,
        deadline: Instant,
    ) -> Result<usize, RuntimeError> {
        self.drain_idle_until(deadline, false, Some(1)).await
    }

    /// All stages use the application's existing deadline. Active leases and unresolved native
    /// builds remain accounted when that deadline expires; there is no replacement or task abort.
    pub async fn shutdown_until(self: &Arc<Self>, deadline: Instant) -> bool {
        self.close();
        loop {
            let _ = self.drain_idle_until(deadline, false, None).await;
            if self.accounting().reserved_bytes == 0 {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep_until(
                (Instant::now() + std::time::Duration::from_millis(5)).min(deadline),
            )
            .await;
        }
    }
}
