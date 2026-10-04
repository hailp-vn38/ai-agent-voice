//! Once-per-process resolution of immutable models that a materialization already verified.

use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use super::{
    DeploymentConfig, ModelError, ResolvedModel, model_fingerprint, prepare_immutable_timed,
};

/// Immutable preparation cost, so a materialization can report which boundary dominated instead of
/// guessing. Verification is timed separately because it is pure read I/O over already-installed
/// bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreparationTimings {
    pub prepare: Duration,
    pub verify: Duration,
}

/// One resolved immutable model plus whether this call had to install and verify it.
#[derive(Clone)]
pub struct PreparedModel {
    pub model: Arc<ResolvedModel>,
    /// False when a result already trusted in this process was reused.
    pub prepared: bool,
    pub timings: PreparationTimings,
}

/// Application-owned cache of models this process has already installed and verified.
///
/// A materialization reads and SHA-256 verifies its whole model today, which for ZeroTTS means
/// hashing gigabytes of ONNX graphs on every runtime load. The key is the manifest fingerprint, so
/// it tracks immutable content rather than a logical provider id: editing the manifest yields a new
/// key and a fresh preparation. Integrity is still enforced at every installation and change
/// boundary; only the redundant re-read of an unchanged, already-trusted tree is skipped.
pub struct PreparedModelCatalog {
    entries: Mutex<CatalogState>,
}

#[derive(Default)]
struct CatalogState {
    models: HashMap<(String, String, String), Arc<ResolvedModel>>,
    preparations: AtomicU64,
    cache_hits: AtomicU64,
}

/// Cumulative cache counters, for diagnostics and regression tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreparedModelCounters {
    pub preparations: u64,
    pub cache_hits: u64,
}

impl Default for PreparedModelCatalog {
    fn default() -> Self {
        Self {
            entries: Mutex::new(CatalogState::default()),
        }
    }
}

impl PreparedModelCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Counters for this catalog only. Two materializers in one process never share a count.
    pub fn counters(&self) -> PreparedModelCounters {
        let state = self
            .entries
            .lock()
            .expect("prepared model catalog poisoned");
        PreparedModelCounters {
            preparations: state.preparations.load(Ordering::Relaxed),
            cache_hits: state.cache_hits.load(Ordering::Relaxed),
        }
    }

    /// Resolves one immutable model, preparing and verifying it only when this process has not
    /// already done so for the same manifest content. A cached entry is only reused while every
    /// pinned artifact is still present, so a removed tree is never silently trusted.
    pub fn resolve(
        &self,
        manifest_path: &Path,
        root: &Path,
        offline: bool,
        identity: &str,
        adapter: &str,
        deployment: &DeploymentConfig,
    ) -> Result<PreparedModel, ModelError> {
        let fingerprint = model_fingerprint(manifest_path, identity, adapter)?;
        let key = (identity.to_owned(), adapter.to_owned(), fingerprint);
        if let Some(model) = {
            let state = self
                .entries
                .lock()
                .expect("prepared model catalog poisoned");
            state
                .models
                .get(&key)
                .filter(|model| model.artifacts().all(|path| path.is_file()))
                .cloned()
        } {
            self.entries
                .lock()
                .expect("prepared model catalog poisoned")
                .cache_hits
                .fetch_add(1, Ordering::Relaxed);
            return Ok(PreparedModel {
                model,
                prepared: false,
                timings: PreparationTimings::default(),
            });
        }
        let preparation =
            prepare_immutable_timed(manifest_path, root, offline, identity, adapter, deployment)?;
        let timings = PreparationTimings {
            prepare: preparation.prepare,
            verify: preparation.verify,
        };
        let model = Arc::new(preparation.model);
        let mut state = self
            .entries
            .lock()
            .expect("prepared model catalog poisoned");
        state.preparations.fetch_add(1, Ordering::Relaxed);
        // Keep whichever entry won the race: both preparations installed the same content.
        let cached = state
            .models
            .entry(key)
            .or_insert_with(|| Arc::clone(&model))
            .clone();
        Ok(PreparedModel {
            model: cached,
            prepared: true,
            timings,
        })
    }
}
