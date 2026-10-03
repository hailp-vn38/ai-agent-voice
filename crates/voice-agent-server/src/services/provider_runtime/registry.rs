use super::{
    ProviderVersion, ResourceLease, RuntimeAccounting, RuntimeError, RuntimeLimits,
    RuntimeMaterializer, RuntimeResource, RuntimeState, lease::LeaseLifetime,
};
use crate::{
    database::DesiredProvider, lifecycle::AdmissionGate, workers::ProviderRuntimeAdmission,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, watch},
    time::{Instant, timeout_at},
};
use tokio_util::sync::CancellationToken;

pub struct ProviderRuntimeManager {
    metrics: Arc<super::RuntimeMetrics>,
    limits: RuntimeLimits,
    builder: Arc<dyn RuntimeMaterializer>,
    gate: Arc<AdmissionGate>,
    stopping: CancellationToken,
    loads: Arc<Semaphore>,
    attempts: Arc<Semaphore>,
    waiters: Arc<Semaphore>,
    registry: Mutex<Registry>,
}
impl std::fmt::Debug for ProviderRuntimeManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRuntimeManager")
            .finish_non_exhaustive()
    }
}
struct Registry {
    entries: HashMap<ProviderVersion, Entry>,
    evicted: std::collections::VecDeque<(Option<super::ResourceKey>, ProviderVersion)>,
    next_generation: u64,
    reserved_bytes: u64,
    build_attempts: u64,
    quotas: HashMap<super::ProviderIdentity, (usize, ProviderRuntimeAdmission)>,
    global_quotas: HashMap<String, (usize, ProviderRuntimeAdmission)>,
}
struct Entry {
    owner: ProviderVersion,
    resource_key: Option<super::ResourceKey>,
    retained: bool,
    physical_admission: Option<ProviderRuntimeAdmission>,
    health_flags: Vec<Arc<std::sync::atomic::AtomicBool>>,
    capabilities: Option<serde_json::Value>,
    generation: u64,
    state: RuntimeState,
    changed: watch::Sender<RuntimeState>,
    resource: Option<Arc<dyn RuntimeResource>>,
    error: Option<RuntimeError>,
    bytes: u64,
    leases: usize,
    waiters: usize,
    idle_since: Instant,
    retry_at: Instant,
    quarantined_load: Option<OwnedSemaphorePermit>,
    quarantined_attempt: Option<OwnedSemaphorePermit>,
    unload_running: bool,
}
impl Entry {
    fn alias(&self) -> Self {
        let (changed, _) = watch::channel(self.state);
        Self {
            owner: self.owner.clone(),
            resource_key: self.resource_key.clone(),
            retained: false,
            physical_admission: self.physical_admission.clone(),
            health_flags: self.health_flags.clone(),
            capabilities: self.capabilities.clone(),
            generation: self.generation,
            state: self.state,
            changed,
            resource: self.resource.clone(),
            error: self.error,
            bytes: 0,
            leases: 0,
            waiters: 0,
            idle_since: Instant::now(),
            retry_at: self.retry_at,
            quarantined_load: None,
            quarantined_attempt: None,
            unload_running: self.unload_running,
        }
    }
    fn healthy(&self) -> bool {
        self.health_flags
            .iter()
            .all(|flag| flag.load(std::sync::atomic::Ordering::Acquire))
    }
}
struct Waiter {
    manager: Arc<ProviderRuntimeManager>,
    version: ProviderVersion,
    generation: u64,
    _capacity: OwnedSemaphorePermit,
}
impl Drop for Waiter {
    fn drop(&mut self) {
        let mut registry = self
            .manager
            .registry
            .lock()
            .expect("runtime registry poisoned");
        if let Some(entry) = registry.entries.get_mut(&self.version)
            && entry.generation == self.generation
        {
            entry.waiters -= 1;
        }
    }
}

impl ProviderRuntimeManager {
    pub fn new(
        limits: RuntimeLimits,
        builder: Arc<dyn RuntimeMaterializer>,
        gate: Arc<AdmissionGate>,
    ) -> Result<Arc<Self>, RuntimeError> {
        limits.validate()?;
        let manager = Arc::new(Self {
            metrics: Arc::new(super::RuntimeMetrics::default()),
            loads: Arc::new(Semaphore::new(limits.max_parallel_loads)),
            attempts: Arc::new(Semaphore::new(
                limits.max_parallel_loads + limits.max_pending_loads,
            )),
            waiters: Arc::new(Semaphore::new(limits.max_waiters)),
            limits,
            builder,
            gate,
            stopping: CancellationToken::new(),
            registry: Mutex::new(Registry {
                entries: HashMap::new(),
                evicted: Default::default(),
                next_generation: 1,
                reserved_bytes: 0,
                build_attempts: 0,
                quotas: HashMap::new(),
                global_quotas: HashMap::new(),
            }),
        });
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let weak = Arc::downgrade(&manager);
            let stopping = manager.stopping.clone();
            let interval = Duration::from_millis(manager.limits.idle_ttl_ms.clamp(1000, 30_000));
            runtime.spawn(async move {
                loop {
                    tokio::select! { _ = stopping.cancelled() => break, _ = tokio::time::sleep(interval) => {} }
                    let Some(manager) = weak.upgrade() else { break };
                    let _ = manager.evict_expired().await;
                }
            });
        }
        Ok(manager)
    }

    pub async fn acquire(
        self: &Arc<Self>,
        snapshot: DesiredProvider,
    ) -> Result<ResourceLease, RuntimeError> {
        let deadline = Instant::now() + Duration::from_millis(self.limits.admission_timeout_ms);
        self.acquire_until(snapshot, deadline).await
    }

    /// Caller stages share this overall deadline. The native attempt outlives cancelled waiters.
    pub async fn acquire_until(
        self: &Arc<Self>,
        snapshot: DesiredProvider,
        deadline: Instant,
    ) -> Result<ResourceLease, RuntimeError> {
        if !bounded_snapshot(&snapshot) || snapshot.id <= 0 || snapshot.revision <= 0 {
            return Err(RuntimeError::Configuration);
        }
        let version = ProviderVersion::database(snapshot.id, snapshot.revision);
        for _ in 0..=self.limits.max_resources {
            match self
                .acquire_version_until(snapshot.clone(), version.clone(), deadline, None)
                .await
            {
                Err(RuntimeError::MemoryPressure) => {
                    if self.evict_pressure_until(deadline).await? == 0 {
                        return Err(RuntimeError::MemoryPressure);
                    }
                }
                outcome => return outcome,
            }
        }
        Err(RuntimeError::MemoryPressure)
    }

    pub async fn acquire_deployment_until(
        self: &Arc<Self>,
        snapshot: DesiredProvider,
        deadline: Instant,
    ) -> Result<ResourceLease, RuntimeError> {
        if !bounded_snapshot(&snapshot) || snapshot.id != 0 || snapshot.revision != 1 {
            return Err(RuntimeError::Configuration);
        }
        let version = ProviderVersion {
            identity: super::ProviderIdentity::Deployment {
                kind: snapshot.kind.clone(),
                key: snapshot.key.clone(),
            },
            revision: 1,
        };
        self.acquire_version_until(snapshot, version, deadline, None)
            .await
    }

    async fn acquire_version_until(
        self: &Arc<Self>,
        snapshot: DesiredProvider,
        version: ProviderVersion,
        deadline: Instant,
        preclaimed_load: Option<OwnedSemaphorePermit>,
    ) -> Result<ResourceLease, RuntimeError> {
        if Instant::now() >= deadline {
            return Err(RuntimeError::Timeout);
        }
        if !self.gate.is_open() || self.stopping.is_cancelled() {
            return Err(RuntimeError::ShuttingDown);
        }
        let capacity = self
            .waiters
            .clone()
            .try_acquire_owned()
            .map_err(|_| RuntimeError::Busy)?;
        // Estimation must be cheap and metadata-only; native work belongs exclusively in build.
        let _timer = self.metrics.timer(super::RuntimePhase::Acquisition);
        let resource_key = self.builder.resource_key(&snapshot)?;
        let bytes = self.builder.estimated_peak_bytes(&snapshot)?;
        if bytes == 0 {
            return Err(RuntimeError::Configuration);
        }
        let total = self.builder.logical_capacity(&snapshot)?;
        if !(1..=4096).contains(&total) {
            return Err(RuntimeError::Configuration);
        }
        let global_capacity = self.builder.global_capacity(&snapshot)?;
        if global_capacity.is_some_and(|value| !(1..=4096).contains(&value)) {
            return Err(RuntimeError::Configuration);
        }
        let (mut changes, generation, start, quota) = {
            let mut registry = self.registry.lock().expect("runtime registry poisoned");
            let now = Instant::now();
            registry.entries.retain(|_, entry| {
                !(entry.state == RuntimeState::Failed
                    && entry.waiters == 0
                    && now >= entry.retry_at)
            });
            let identities: std::collections::HashSet<_> = registry
                .entries
                .keys()
                .map(|version| version.identity.clone())
                .collect();
            registry
                .quotas
                .retain(|identity, _| identities.contains(identity));
            if registry
                .quotas
                .get(&version.identity)
                .is_some_and(|(existing, _)| *existing != total)
            {
                return Err(RuntimeError::Configuration);
            }
            if let Some(capacity) = global_capacity
                && registry
                    .global_quotas
                    .get(&snapshot.kind)
                    .is_some_and(|(existing, _)| *existing != capacity)
            {
                return Err(RuntimeError::Configuration);
            }
            let start = if !registry.entries.contains_key(&version) {
                if registry.entries.len() >= self.limits.max_version_entries {
                    let mut idle_aliases: Vec<_> = registry
                        .entries
                        .iter()
                        .filter(|(v, e)| {
                            **v != e.owner && !e.retained && e.leases == 0 && e.waiters == 0
                        })
                        .map(|(v, e)| (v.clone(), e.idle_since))
                        .collect();
                    idle_aliases.sort_by_key(|(_, idle)| *idle);
                    if let Some((alias, _)) = idle_aliases.first() {
                        registry.entries.remove(alias);
                    }
                    if registry.entries.len() >= self.limits.max_version_entries {
                        return Err(RuntimeError::Busy);
                    }
                }
                let shared = resource_key
                    .as_ref()
                    .and_then(|key| {
                        registry
                            .entries
                            .iter()
                            .find(|(v, e)| **v == e.owner && e.resource_key.as_ref() == Some(key))
                    })
                    .map(|(_, entry)| entry.alias());
                if let Some(alias) = shared {
                    registry.entries.insert(version.clone(), alias);
                    None
                } else {
                    let resources = registry
                        .entries
                        .values()
                        .filter(|entry| entry.bytes > 0)
                        .count();
                    if resources >= self.limits.max_resources
                        || bytes
                            > self
                                .limits
                                .max_resident_bytes
                                .saturating_sub(registry.reserved_bytes)
                    {
                        return Err(RuntimeError::MemoryPressure);
                    }
                    let attempt = self
                        .attempts
                        .clone()
                        .try_acquire_owned()
                        .map_err(|_| RuntimeError::Busy)?;
                    if registry.evicted.iter().any(|(key, old)| {
                        resource_key
                            .as_ref()
                            .map_or(old == &version, |current| key.as_ref() == Some(current))
                    }) {
                        self.metrics.increment(super::RuntimeCounter::Reload);
                    }
                    let generation = registry.next_generation;
                    registry.next_generation += 1;
                    let (changed, _) = watch::channel(RuntimeState::Queued);
                    registry.reserved_bytes += bytes;
                    registry.entries.insert(
                        version.clone(),
                        Entry {
                            owner: version.clone(),
                            resource_key: resource_key.clone(),
                            retained: false,
                            physical_admission: None,
                            health_flags: Vec::new(),
                            capabilities: None,
                            generation,
                            state: RuntimeState::Queued,
                            changed,
                            resource: None,
                            error: None,
                            bytes,
                            leases: 0,
                            waiters: 0,
                            idle_since: now,
                            retry_at: now,
                            quarantined_load: None,
                            quarantined_attempt: None,
                            unload_running: false,
                        },
                    );
                    Some(attempt)
                }
            } else {
                None
            };
            let (existing_total, quota) = registry
                .quotas
                .entry(version.identity.clone())
                .or_insert_with(|| (total, ProviderRuntimeAdmission::new(total, 1)));
            debug_assert_eq!(*existing_total, total);
            let mut quota = quota.clone();
            if let Some(capacity) = global_capacity {
                let (existing, global) = registry
                    .global_quotas
                    .entry(snapshot.kind.clone())
                    .or_insert_with(|| (capacity, ProviderRuntimeAdmission::new(capacity, 1)));
                debug_assert_eq!(*existing, capacity);
                quota = quota.composed(global);
            }
            let entry = registry
                .entries
                .get_mut(&version)
                .expect("entry just registered");
            self.metrics.increment(if start.is_some() {
                super::RuntimeCounter::Miss
            } else if entry.state == RuntimeState::Ready {
                super::RuntimeCounter::Hit
            } else {
                super::RuntimeCounter::Coalesced
            });
            entry.waiters += 1;
            (entry.changed.subscribe(), entry.generation, start, quota)
        };
        let _waiter = Waiter {
            manager: Arc::clone(self),
            version: version.clone(),
            generation,
            _capacity: capacity,
        };
        if let Some(attempt) = start {
            let manager = Arc::clone(self);
            let version = version.clone();
            let build_snapshot = snapshot.clone();
            let physical_quota = if resource_key.is_some() {
                ProviderRuntimeAdmission::new(total, 1)
            } else {
                quota.clone()
            };
            tokio::spawn(async move {
                manager
                    .build(
                        version,
                        generation,
                        build_snapshot,
                        physical_quota,
                        attempt,
                        preclaimed_load,
                    )
                    .await;
            });
        }
        let wait = async {
            loop {
                {
                    let mut registry = self.registry.lock().expect("runtime registry poisoned");
                    let entry = registry
                        .entries
                        .get_mut(&version)
                        .ok_or(RuntimeError::Unavailable)?;
                    if entry.generation != generation {
                        return Err(RuntimeError::Unavailable);
                    }
                    if entry.state == RuntimeState::Ready && !entry.healthy() {
                        entry.state = RuntimeState::Quarantined;
                        entry.error = Some(RuntimeError::Quarantined);
                        entry.changed.send_replace(entry.state);
                    }
                    match entry.state {
                        RuntimeState::Ready => {
                            if !self.gate.is_open() {
                                return Err(RuntimeError::ShuttingDown);
                            }
                            let resource =
                                entry.resource.clone().ok_or(RuntimeError::Unavailable)?;
                            entry.leases += 1;
                            return Ok(ResourceLease(Arc::new(LeaseLifetime {
                                manager: Arc::clone(self),
                                snapshot: snapshot.clone(),
                                quota: quota.clone(),
                                version: version.clone(),
                                generation,
                                resource,
                            })));
                        }
                        RuntimeState::Failed | RuntimeState::Quarantined => {
                            return Err(entry.error.unwrap_or(RuntimeError::Unavailable));
                        }
                        RuntimeState::Draining | RuntimeState::Cold => {
                            return Err(RuntimeError::Unavailable);
                        }
                        RuntimeState::Queued | RuntimeState::Loading => {}
                    }
                }
                tokio::select! {
                    _ = self.stopping.cancelled() => return Err(RuntimeError::ShuttingDown),
                    changed = changes.changed() => if changed.is_err() { return Err(RuntimeError::Unavailable); },
                }
            }
        };
        timeout_at(deadline, wait)
            .await
            .map_err(|_| RuntimeError::Timeout)?
    }

    async fn build(
        self: Arc<Self>,
        version: ProviderVersion,
        generation: u64,
        snapshot: DesiredProvider,
        quota: ProviderRuntimeAdmission,
        attempt: OwnedSemaphorePermit,
        preclaimed_load: Option<OwnedSemaphorePermit>,
    ) {
        let speculative = preclaimed_load.is_some();
        let load = if preclaimed_load.is_some() {
            preclaimed_load
        } else {
            tokio::select! {
                permit = self.loads.clone().acquire_owned() => permit.ok(),
                _ = self.stopping.cancelled() => None,
            }
        };
        if !self.gate.is_open() || load.is_none() {
            self.complete(
                &version,
                generation,
                Err(RuntimeError::ShuttingDown),
                None,
                Some(attempt),
            );
            return;
        }
        {
            let mut registry = self.registry.lock().expect("runtime registry poisoned");
            registry.build_attempts += 1;
            for entry in registry
                .entries
                .values_mut()
                .filter(|entry| entry.owner == version)
            {
                if entry.owner == version && entry.bytes > 0 {
                    self.metrics
                        .observe(super::RuntimePhase::Queue, entry.idle_since.elapsed());
                }
                entry.state = RuntimeState::Loading;
                entry.changed.send_replace(RuntimeState::Loading);
            }
        }
        let builder = Arc::clone(&self.builder);
        let manager = Arc::clone(&self);
        let native_version = version.clone();
        let runtime = tokio::runtime::Handle::current();
        // A bounded native thread owns completion and permits directly. Aborting an async
        // executor cannot lose completion, and Tokio shutdown does not wait past our deadline.
        let spawn = std::thread::Builder::new()
            .name("provider-load".into())
            .spawn(move || {
                let _context = runtime.enter();
                let _timer = manager.metrics.timer(super::RuntimePhase::Build);
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if !manager.gate.is_open() {
                        return Err(RuntimeError::ShuttingDown);
                    }
                    if speculative {
                        builder.prepare_artifacts(&snapshot)?;
                    }
                    builder.build(&snapshot, quota)
                }))
                .unwrap_or(Err(RuntimeError::Quarantined));
                manager.complete(&native_version, generation, outcome, load, Some(attempt));
            });
        if spawn.is_err() {
            self.complete(
                &version,
                generation,
                Err(RuntimeError::Unavailable),
                None,
                None,
            );
        }
    }

    fn complete(
        &self,
        version: &ProviderVersion,
        generation: u64,
        outcome: Result<Arc<dyn RuntimeResource>, RuntimeError>,
        load: Option<OwnedSemaphorePermit>,
        attempt: Option<OwnedSemaphorePermit>,
    ) {
        let metadata = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            outcome.as_ref().ok().map(|resource| {
                (
                    resource.capabilities(),
                    resource.health_flags(),
                    resource.resource_key(),
                    resource.physical_admission(),
                    resource.readiness(),
                )
            })
        }));
        let metadata_failed = metadata.is_err();
        let (capabilities, health_flags, actual_key, physical_admission, readiness) = metadata
            .ok()
            .flatten()
            .unwrap_or_else(|| (None, Vec::new(), None, None, Default::default()));
        if !metadata_failed {
            self.metrics
                .observe(super::RuntimePhase::WorkerInit, readiness.initialization);
            self.metrics
                .observe(super::RuntimePhase::Warmup, readiness.warmup);
        }
        let mut registry = self.registry.lock().expect("runtime registry poisoned");
        let Some(entry) = registry.entries.get_mut(version) else {
            return;
        };
        if entry.generation != generation {
            return;
        }
        let mut released = 0;
        match outcome {
            Ok(resource) => {
                let invalid =
                    metadata_failed || (actual_key.is_some() && actual_key != entry.resource_key);
                self.metrics.increment(if invalid {
                    super::RuntimeCounter::BuildFailure
                } else {
                    super::RuntimeCounter::BuildSuccess
                });
                entry.physical_admission = physical_admission;
                entry.health_flags = health_flags;
                entry.capabilities = capabilities;
                entry.resource = Some(resource);
                entry.state = if invalid {
                    entry.error = Some(RuntimeError::Quarantined);
                    RuntimeState::Quarantined
                } else {
                    RuntimeState::Ready
                };
                entry.idle_since = Instant::now();
            }
            Err(RuntimeError::Quarantined) => {
                self.metrics.increment(super::RuntimeCounter::BuildFailure);
                entry.state = RuntimeState::Quarantined;
                entry.error = Some(RuntimeError::Quarantined);
                entry.quarantined_load = load;
                entry.quarantined_attempt = attempt;
            }
            Err(error) => {
                self.metrics.increment(super::RuntimeCounter::BuildFailure);
                entry.state = RuntimeState::Failed;
                entry.error = Some(error);
                entry.retry_at =
                    Instant::now() + Duration::from_millis(self.limits.failure_cooldown_ms);
                released = entry.bytes;
                entry.bytes = 0;
            }
        }
        entry.changed.send_replace(entry.state);
        let metadata = (
            entry.state,
            entry.resource.clone(),
            entry.error,
            entry.retry_at,
            entry.capabilities.clone(),
            entry.health_flags.clone(),
            entry.physical_admission.clone(),
        );
        for alias in registry
            .entries
            .values_mut()
            .filter(|entry| entry.owner == *version && entry.bytes == 0)
        {
            alias.state = metadata.0;
            alias.resource = metadata.1.clone();
            alias.error = metadata.2;
            alias.retry_at = metadata.3;
            alias.capabilities = metadata.4.clone();
            alias.health_flags = metadata.5.clone();
            alias.physical_admission = metadata.6.clone();
            alias.changed.send_replace(alias.state);
        }
        registry.reserved_bytes -= released;
    }

    pub(super) fn release(&self, version: &ProviderVersion, generation: u64) {
        let mut registry = self.registry.lock().expect("runtime registry poisoned");
        if let Some(entry) = registry.entries.get_mut(version)
            && entry.generation == generation
        {
            entry.leases -= 1;
            if entry.leases == 0 {
                entry.idle_since = Instant::now();
            }
        }
    }

    pub(crate) fn prewarm_capacity(&self) -> usize {
        self.limits.max_pending_loads
    }
    pub(crate) fn prewarm_resource_key(
        &self,
        snapshot: &DesiredProvider,
    ) -> Result<Option<super::ResourceKey>, RuntimeError> {
        self.builder.resource_key(snapshot)
    }
    pub(crate) fn stopping_token(&self) -> CancellationToken {
        self.stopping.clone()
    }
    /// Speculation never queues for loader capacity: explicit FIFO waiters are served first.
    pub(crate) async fn acquire_speculative(
        self: &Arc<Self>,
        snapshot: DesiredProvider,
    ) -> Result<ResourceLease, RuntimeError> {
        if self.accounting().waiters > 0 {
            return Err(RuntimeError::Busy);
        }
        let load = self
            .loads
            .clone()
            .try_acquire_owned()
            .map_err(|_| RuntimeError::Busy)?;
        let version = ProviderVersion::database(snapshot.id, snapshot.revision);
        if !bounded_snapshot(&snapshot) || snapshot.id <= 0 || snapshot.revision <= 0 {
            return Err(RuntimeError::Configuration);
        }
        self.acquire_version_until(snapshot, version, self.admission_deadline(), Some(load))
            .await
    }

    pub fn retain_deployment(&self, version: &ProviderVersion) -> Result<(), RuntimeError> {
        if !matches!(version.identity, super::ProviderIdentity::Deployment { .. }) {
            return Err(RuntimeError::Configuration);
        }
        let mut registry = self.registry.lock().expect("runtime registry poisoned");
        let entry = registry
            .entries
            .get_mut(version)
            .ok_or(RuntimeError::Unavailable)?;
        if entry.state != RuntimeState::Ready || !entry.healthy() {
            return Err(RuntimeError::Unavailable);
        }
        entry.retained = true;
        Ok(())
    }
    pub fn deployment_ready(&self, kind: &str, key: &str) -> bool {
        let version = ProviderVersion {
            identity: super::ProviderIdentity::Deployment {
                kind: kind.into(),
                key: key.into(),
            },
            revision: 1,
        };
        self.registry
            .lock()
            .expect("runtime registry poisoned")
            .entries
            .get(&version)
            .is_some_and(|entry| entry.state == RuntimeState::Ready && entry.healthy())
    }

    pub fn admission_deadline(&self) -> Instant {
        Instant::now() + Duration::from_millis(self.limits.admission_timeout_ms)
    }

    /// Metadata-only inspection. It neither resolves credentials nor refreshes cache usage.
    pub fn inspect(&self, provider_id: i64, desired_revision: i64) -> super::RuntimeInspection {
        let registry = self.registry.lock().expect("runtime registry poisoned");
        let desired = registry
            .entries
            .get(&ProviderVersion::database(provider_id, desired_revision));
        let mut ready_revisions: Vec<_> = registry
            .entries
            .iter()
            .filter(|(version, entry)| {
                version.identity == super::ProviderIdentity::Database(provider_id)
                    && entry.state == RuntimeState::Ready
                    && entry.healthy()
            })
            .map(|(version, _)| version.revision)
            .collect();
        ready_revisions.sort_unstable();
        super::RuntimeInspection {
            desired_revision,
            desired_state: desired.map_or(RuntimeState::Cold, |entry| {
                if entry.state == RuntimeState::Ready && !entry.healthy() {
                    RuntimeState::Quarantined
                } else {
                    entry.state
                }
            }),
            ready_revisions,
            can_prepare: self.gate.is_open() && !self.stopping.is_cancelled(),
            failure_code: desired.and_then(|entry| {
                if entry.state == RuntimeState::Ready && !entry.healthy() {
                    Some(RuntimeError::Quarantined.to_string())
                } else {
                    entry.error.map(|error| error.to_string())
                }
            }),
        }
    }

    pub fn ready_capabilities(&self, provider_id: i64, revision: i64) -> Option<serde_json::Value> {
        let registry = self.registry.lock().expect("runtime registry poisoned");
        registry
            .entries
            .get(&ProviderVersion::database(provider_id, revision))
            .filter(|entry| entry.state == RuntimeState::Ready && entry.healthy())
            .and_then(|entry| entry.capabilities.clone())
    }

    pub(super) fn begin_drains(
        &self,
        expired_only: bool,
        max_drains: Option<usize>,
    ) -> Vec<IdleDrain> {
        let mut registry = self.registry.lock().expect("runtime registry poisoned");
        let now = Instant::now();
        let mut candidates: Vec<_> = registry
            .entries
            .iter()
            .filter(|(version, entry)| {
                **version == entry.owner
                    && entry.resource.is_some()
                    && !entry.unload_running
                    && matches!(entry.state, RuntimeState::Ready | RuntimeState::Quarantined)
                    && registry
                        .entries
                        .values()
                        .filter(|alias| alias.owner == **version)
                        .all(|alias| {
                            alias.leases == 0
                                && alias.waiters == 0
                                && (!alias.retained || self.stopping.is_cancelled())
                                && (!expired_only
                                    || now.duration_since(alias.idle_since)
                                        >= Duration::from_millis(self.limits.idle_ttl_ms))
                        })
            })
            .map(|(version, _)| {
                let idle = registry
                    .entries
                    .values()
                    .filter(|entry| entry.owner == *version)
                    .map(|entry| entry.idle_since)
                    .max()
                    .expect("owner exists");
                (version.clone(), idle)
            })
            .collect();
        candidates.sort_by_key(|(_, idle)| *idle);
        if let Some(limit) = max_drains {
            candidates.truncate(limit);
        }
        let mut drains = Vec::with_capacity(candidates.len());
        for (version, _) in candidates {
            let entry = registry.entries.get(&version).expect("owner exists");
            let drain = IdleDrain {
                version: version.clone(),
                generation: entry.generation,
                resource: entry.resource.clone().expect("resource exists"),
            };
            for alias in registry
                .entries
                .values_mut()
                .filter(|entry| entry.owner == version)
            {
                alias.state = RuntimeState::Draining;
                alias.unload_running = true;
                alias.changed.send_replace(RuntimeState::Draining);
            }
            drains.push(drain);
        }
        drains
    }

    pub(super) fn finish_drain(
        &self,
        version: &ProviderVersion,
        generation: u64,
        acknowledged: bool,
    ) {
        let mut registry = self.registry.lock().expect("runtime registry poisoned");
        let Some(entry) = registry.entries.get_mut(version) else {
            return;
        };
        if entry.generation != generation {
            return;
        }
        if !acknowledged {
            for entry in registry
                .entries
                .values_mut()
                .filter(|entry| entry.owner == *version)
            {
                entry.state = RuntimeState::Quarantined;
                entry.error = Some(RuntimeError::Quarantined);
                entry.unload_running = false;
                entry.changed.send_replace(RuntimeState::Quarantined);
            }
            return;
        }
        let bytes = entry.bytes;
        let key = entry.resource_key.clone();
        if registry.evicted.len() >= self.limits.max_version_entries {
            registry.evicted.pop_front();
        }
        registry.evicted.push_back((key, version.clone()));
        registry.entries.retain(|_, entry| {
            if entry.owner == *version {
                entry.changed.send_replace(RuntimeState::Cold);
                false
            } else {
                true
            }
        });
        registry.reserved_bytes -= bytes;
        self.metrics.increment(super::RuntimeCounter::Eviction);
    }

    pub(super) fn drain_timeout(&self, version: &ProviderVersion, generation: u64) {
        let mut registry = self.registry.lock().expect("runtime registry poisoned");
        for entry in registry.entries.values_mut().filter(|entry| {
            entry.owner == *version && entry.generation == generation && entry.unload_running
        }) {
            entry.state = RuntimeState::Quarantined;
            entry.error = Some(RuntimeError::Quarantined);
            entry.changed.send_replace(RuntimeState::Quarantined);
        }
    }

    pub fn close(&self) {
        self.gate.close();
        self.stopping.cancel();
    }

    pub fn state(&self, version: &ProviderVersion) -> RuntimeState {
        self.registry
            .lock()
            .expect("runtime registry poisoned")
            .entries
            .get(version)
            .map_or(RuntimeState::Cold, |entry| entry.state)
    }
    pub fn metrics(&self) -> &Arc<super::RuntimeMetrics> {
        &self.metrics
    }

    /// Bounded identities for joining aggregate status against current desired rows.
    pub(crate) fn tracked_database_ids(&self) -> Vec<i64> {
        let registry = self.registry.lock().expect("runtime registry poisoned");
        let mut ids: Vec<_> = registry
            .entries
            .keys()
            .filter_map(|version| match version.identity {
                super::ProviderIdentity::Database(id) => Some(id),
                _ => None,
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }
    pub fn accounting(&self) -> RuntimeAccounting {
        let registry = self.registry.lock().expect("runtime registry poisoned");
        RuntimeAccounting {
            version_entries: registry.entries.len(),
            logical_inference_usage: registry
                .quotas
                .values()
                .map(|(_, quota)| quota.view_usage())
                .sum(),
            global_inference_usage: registry
                .global_quotas
                .values()
                .map(|(_, quota)| quota.view_usage())
                .sum(),
            physical_inference_usage: registry
                .entries
                .iter()
                .filter(|(version, entry)| **version == entry.owner)
                .filter_map(|(_, entry)| entry.physical_admission.as_ref())
                .map(|quota| quota.view_usage())
                .sum(),
            reserved_build_bytes: registry
                .entries
                .values()
                .filter(|entry| matches!(entry.state, RuntimeState::Queued | RuntimeState::Loading))
                .map(|entry| entry.bytes)
                .sum(),
            draining_bytes: registry
                .entries
                .values()
                .filter(|entry| entry.state == RuntimeState::Draining)
                .map(|entry| entry.bytes)
                .sum(),
            quarantined_bytes: registry
                .entries
                .values()
                .filter(|entry| entry.state == RuntimeState::Quarantined)
                .map(|entry| entry.bytes)
                .sum(),
            reserved_bytes: registry.reserved_bytes,
            resources: registry
                .entries
                .values()
                .filter(|entry| entry.bytes > 0)
                .count(),
            active_leases: registry.entries.values().map(|entry| entry.leases).sum(),
            waiters: registry.entries.values().map(|entry| entry.waiters).sum(),
            build_attempts: registry.build_attempts,
        }
    }
}

pub(super) struct IdleDrain {
    pub version: ProviderVersion,
    pub generation: u64,
    pub resource: Arc<dyn RuntimeResource>,
}

fn bounded_snapshot(snapshot: &DesiredProvider) -> bool {
    !snapshot.key.is_empty()
        && snapshot.key.len() <= 128
        && snapshot.kind.len() <= 16
        && snapshot.adapter.len() <= 64
        && snapshot.config_json.len() <= 65_536
        && snapshot
            .secret_ref
            .as_ref()
            .is_none_or(|value| value.len() <= 256)
}
