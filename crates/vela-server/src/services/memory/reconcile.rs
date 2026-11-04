//! Convergence after crashes and edits made outside the system. There is no file watcher: this runs
//! at startup and on an Admin rebuild.

use super::{
    Inner,
    fact_file::render,
    index,
    read::{
        blocking, publish_deployment, publish_from_disk, read_state, visible_facts,
        write_summary_file,
    },
    types::{
        FactOrigin, MemoryError, MemoryScope, ReconcileAllReport, ReconcileReport, ScopeTarget,
    },
    vault::{self, Vault},
};
use chrono::Utc;
use std::{collections::HashSet, fs, io, sync::Arc};

pub(super) async fn reconcile(
    inner: &Arc<Inner>,
    scope: &MemoryScope,
) -> Result<ReconcileReport, MemoryError> {
    let ScopeTarget::Device { dir } = &scope.0 else {
        return Err(MemoryError::ScopeDenied);
    };
    let entry = inner.registry.entry(dir);
    let _writer = entry.writer.lock().await;
    let paths = inner.vault.device(dir);

    // A Device without a vault stays without one: startup runs this for every Device.
    if !paths.dir.exists() {
        publish_from_disk(inner, dir, &entry).await?;
        return Ok(ReconcileReport::default());
    }

    let task_inner = inner.clone();
    let task_dir = dir.clone();
    let report = blocking(move || {
        let paths = task_inner.vault.device(&task_dir);
        let mut report = ReconcileReport {
            temp_files_removed: vault::remove_temp_files(&paths.dir)?
                + vault::remove_temp_files(&paths.facts)?,
            ..ReconcileReport::default()
        };
        // A commit may have reported a failed sync. `MEMORY.md` below covers the Device directory.
        vault::sync_if_present(&paths.facts)?;
        let state = read_state(&paths, &task_inner.key, &task_dir)?;
        report.quarantined = state.quarantined.len();
        report.facts = state.live.len();
        for residual in &state.residual {
            vault::remove_durable(&residual.path)?;
            report.tombstones_applied += 1;
        }
        for mut file in state.live {
            if file.is_owner_edited() {
                file.fact.revision += 1;
                file.fact.origin = FactOrigin::OwnerEdited;
                file.fact.updated_at = Utc::now();
                file.refresh_hash();
                vault::write_atomic(&file.path, render(&file)?.as_bytes())?;
                report.owner_edited += 1;
            }
        }
        Ok(report)
    })
    .await?;

    let task_inner = inner.clone();
    let task_dir = dir.clone();
    let state = blocking(move || {
        read_state(
            &task_inner.vault.device(&task_dir),
            &task_inner.key,
            &task_dir,
        )
    })
    .await?;
    if index::rebuild(&paths.index, &visible_facts(&state))
        .await
        .is_err()
    {
        tracing::warn!(event = "memory_index_rebuild_failed");
        let path = paths.index.clone();
        blocking(move || index::discard(&path)).await?;
    }

    let derived = publish_from_disk(inner, dir, &entry).await?;
    blocking(move || write_summary_file(&paths, &derived.summary_file)).await?;
    Ok(report)
}

/// Directories under `devices/` that no `known` scope owns.
fn find_orphans(inner: &Inner, known: &HashSet<String>) -> io::Result<Vec<String>> {
    let entries = match fs::read_dir(inner.vault.devices_dir()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut orphans = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() && !known.contains(&name) {
            orphans.push(name);
        }
    }
    orphans.sort();
    Ok(orphans)
}

pub(super) async fn reconcile_all(inner: &Arc<Inner>, known: &[MemoryScope]) -> ReconcileAllReport {
    let mut report = ReconcileAllReport::default();
    let mut known_dirs = HashSet::new();
    for scope in known {
        match &scope.0 {
            ScopeTarget::Device { .. } => {
                known_dirs.insert(Vault::scope_key(scope));
                match reconcile(inner, scope).await {
                    Ok(_) => report.reconciled += 1,
                    Err(error) => {
                        report.failed += 1;
                        tracing::warn!(event = "memory_reconcile_failed", code = error.code());
                    }
                }
            }
            ScopeTarget::Deployment => {
                let key = Vault::scope_key(scope);
                let entry = inner.registry.entry(&key);
                let _writer = entry.writer.lock().await;
                let dir = inner.vault.deployment_dir();
                let cleaned = blocking(move || {
                    vault::remove_temp_files(&dir)?;
                    vault::sync_if_present(&dir)
                })
                .await;
                if let Err(error) = cleaned {
                    report.failed += 1;
                    tracing::warn!(event = "memory_reconcile_failed", code = error.code());
                }
                let _ = publish_deployment(inner, &entry).await;
            }
        }
    }
    let task_inner = inner.clone();
    match blocking(move || find_orphans(&task_inner, &known_dirs)).await {
        Ok(orphans) => {
            if !orphans.is_empty() {
                tracing::info!(event = "memory_orphan_dirs", count = orphans.len());
            }
            report.orphans = orphans;
        }
        Err(error) => {
            report.failed += 1;
            tracing::warn!(event = "memory_reconcile_failed", code = error.code());
        }
    }
    report
}
