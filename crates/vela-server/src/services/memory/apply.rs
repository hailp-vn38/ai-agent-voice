//! The write order of every `MemoryChange`. Before the commit point an error means nothing changed;
//! after it `apply` always succeeds and reports unfinished work as `cleanup_pending`.

use super::{
    Inner,
    fact_file::{self, FactFile, ProfileFile, read_profile, render, render_profile},
    guard::guard,
    index,
    read::{
        Derived, VaultState, blocking, publish, publish_deployment, publish_from_disk, read_state,
        same_body, visible_facts, write_summary_file,
    },
    scopes::ScopeEntry,
    tombstone::fingerprint,
    types::{
        FactId, FactKind, FactOrigin, FactStatus, MemoryActor, MemoryChange, MemoryContext,
        MemoryError, MemoryFact, MemoryMode, MemoryReceipt, MemoryScope, Provenance, ReceiptAction,
        ScopeTarget,
    },
    vault::{self, DevicePaths, Vault},
};
use chrono::Utc;
use serde_yaml::Mapping;
use std::{io, path::PathBuf, sync::Arc};

const MAX_SLUG_CHARS: usize = 64;

/// Steps a test can interrupt. Each fires just before its step runs.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FailPoint {
    Commit,
    Index,
    Publish,
    Summary,
    Cleanup,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(super) enum FailAction {
    /// Stops everything here, as a process crash would. Only tests can leave `apply` this way
    /// after the commit point.
    Crash,
    /// The step fails and `apply` carries on.
    Error,
    Stall(std::time::Duration),
}

/// `Ok(true)` runs the step, `Ok(false)` makes it fail, `Err` is the simulated crash.
#[cfg(test)]
async fn checkpoint(inner: &Inner, point: FailPoint) -> Result<bool, MemoryError> {
    let armed = {
        let mut slot = inner.fail_point.lock().unwrap_or_else(|e| e.into_inner());
        match *slot {
            Some((armed, action)) if armed == point => {
                *slot = None;
                Some(action)
            }
            _ => None,
        }
    };
    match armed {
        None => Ok(true),
        Some(FailAction::Crash) => Err(MemoryError::Unavailable),
        Some(FailAction::Error) => Ok(false),
        Some(FailAction::Stall(duration)) => {
            tokio::time::sleep(duration).await;
            Ok(true)
        }
    }
}

macro_rules! step {
    ($inner:expr, $point:ident) => {{
        #[cfg(test)]
        {
            checkpoint($inner, FailPoint::$point).await?
        }
        #[cfg(not(test))]
        {
            true
        }
    }};
}

pub(super) async fn apply(
    inner: Arc<Inner>,
    scope: MemoryScope,
    actor: MemoryActor,
    change: MemoryChange,
) -> Result<MemoryReceipt, MemoryError> {
    authorize(&scope, &actor, &change)?;
    let key = Vault::scope_key(&scope);
    let entry = inner.registry.entry(&key);
    let _writer = entry.writer.lock().await;
    match (&scope.0, change) {
        (
            ScopeTarget::Deployment,
            MemoryChange::SetProfile {
                body,
                expected_revision,
            },
        ) => set_profile(&inner, &key, body, expected_revision).await,
        (ScopeTarget::Device { dir }, change) => {
            let writer = DeviceWriter {
                paths: inner.vault.device(dir),
                inner: &inner,
                entry: &entry,
                dir,
            };
            writer.run(actor, change).await
        }
        (ScopeTarget::Deployment, _) => Err(MemoryError::ScopeDenied),
    }
}

/// The only place permissions are decided.
fn authorize(
    scope: &MemoryScope,
    actor: &MemoryActor,
    change: &MemoryChange,
) -> Result<(), MemoryError> {
    let is_profile = matches!(change, MemoryChange::SetProfile { .. });
    if matches!(scope.0, ScopeTarget::Deployment) != is_profile {
        return Err(MemoryError::ScopeDenied);
    }
    let MemoryActor::VoiceUser { mode, intent, .. } = actor else {
        return Ok(());
    };
    if *mode == MemoryMode::Off {
        return Err(MemoryError::Disabled);
    }
    let requested = match change {
        MemoryChange::Remember { .. } => intent.remember,
        MemoryChange::Correct { .. } => intent.correct,
        MemoryChange::Forget { .. } => intent.forget,
        MemoryChange::Discard { .. }
        | MemoryChange::SetPinned { .. }
        | MemoryChange::SetProfile { .. }
        | MemoryChange::PurgeScope => return Err(MemoryError::ScopeDenied),
    };
    if *mode == MemoryMode::Read {
        return Err(MemoryError::ReadOnly);
    }
    if !requested {
        return Err(MemoryError::RequiresExplicitRequest);
    }
    Ok(())
}

/// Titles and hooks are one line. Besides reading better in the summary, this keeps a value from
/// forming a `---` line that would end the frontmatter early and quarantine the file.
fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn check_revision(fact: &MemoryFact, expected: u64) -> Result<(), MemoryError> {
    if fact.revision == expected {
        Ok(())
    } else {
        Err(MemoryError::RevisionConflict)
    }
}

fn find_live(state: VaultState, id: &FactId) -> Result<(FactFile, VaultState), MemoryError> {
    let mut state = state;
    let position = state
        .live
        .iter()
        .position(|file| &file.fact.id == id)
        .ok_or(MemoryError::NotFound)?;
    let file = state.live.remove(position);
    Ok((file, state))
}

fn write_fact(file: &FactFile) -> io::Result<vault::Committed> {
    vault::commit_atomic(&file.path, render(file)?.as_bytes())
}

/// Never overwrites: a taken slug gets a suffix from the id.
fn free_fact_path(paths: &DevicePaths, title: &str, id: &FactId) -> PathBuf {
    let slug = llm_wiki::slugify(title, MAX_SLUG_CHARS);
    let path = paths.facts.join(format!("{slug}.md"));
    if path.exists() {
        paths.facts.join(format!("{slug}-{}.md", id.slug_suffix()))
    } else {
        path
    }
}

enum IndexChange {
    Upsert(MemoryFact),
    Remove(FactId),
    Keep,
}

struct DeviceWriter<'a> {
    inner: &'a Arc<Inner>,
    entry: &'a ScopeEntry,
    paths: DevicePaths,
    dir: &'a str,
}

impl DeviceWriter<'_> {
    async fn run(
        &self,
        actor: MemoryActor,
        change: MemoryChange,
    ) -> Result<MemoryReceipt, MemoryError> {
        match change {
            MemoryChange::Remember {
                title,
                hook,
                body,
                kind,
                source_ref,
            } => {
                self.remember(actor, title, hook, body, kind, source_ref)
                    .await
            }
            MemoryChange::Correct {
                id,
                expected_revision,
                title,
                hook,
                body,
            } => {
                self.correct(actor, id, expected_revision, title, hook, body)
                    .await
            }
            MemoryChange::Discard {
                id,
                expected_revision,
            } => self.discard(id, expected_revision).await,
            MemoryChange::Forget {
                id,
                expected_revision,
            } => self.forget(id, expected_revision).await,
            MemoryChange::SetPinned { id, pinned } => self.set_pinned(id, pinned).await,
            MemoryChange::PurgeScope => self.purge().await,
            MemoryChange::SetProfile { .. } => Err(MemoryError::ScopeDenied),
        }
    }

    async fn state(&self) -> Result<VaultState, MemoryError> {
        let inner = self.inner.clone();
        let dir = self.dir.to_string();
        blocking(move || read_state(&inner.vault.device(&dir), &inner.key, &dir)).await
    }

    fn fingerprint(&self, body: &str) -> String {
        fingerprint(&self.inner.key, self.dir, body)
    }

    /// Lets an explicit write reuse forgotten content. Files a crashed Forget left behind go first:
    /// lifting the tombstone while they exist would bring them back.
    async fn lift_tombstone(&self, state: &VaultState, body: &str) -> Result<(), MemoryError> {
        let fp = self.fingerprint(body);
        if !state.tombstones.contains(&fp) {
            return Ok(());
        }
        let leftovers: Vec<PathBuf> = state
            .residual
            .iter()
            .filter(|file| self.fingerprint(&file.fact.body) == fp)
            .map(|file| file.path.clone())
            .collect();
        let mut tombstones = state.tombstones.clone();
        tombstones.remove(&fp);
        let path = self.paths.tombstones.clone();
        blocking(move || {
            for leftover in &leftovers {
                vault::remove_durable(leftover)?;
            }
            tombstones.write(&path)
        })
        .await
    }

    async fn remember(
        &self,
        actor: MemoryActor,
        title: String,
        hook: String,
        body: String,
        kind: FactKind,
        source_ref: Option<String>,
    ) -> Result<MemoryReceipt, MemoryError> {
        let title = single_line(&title);
        let hook = single_line(&hook);
        let body = fact_file::canonical_text(&body);
        guard(&title, &hook, &body, self.inner.max_fact_body_chars)?;
        if let Some(source_ref) = &source_ref {
            guard("", source_ref, "", self.inner.max_fact_body_chars)?;
        }
        let state = self.state().await?;
        // Only what `recall` can return: a receipt for an invisible fact would break invariant 1.
        if let Some(existing) = visible_facts(&state)
            .into_iter()
            .find(|fact| same_body(&fact.body, &body))
        {
            return Ok(receipt(ReceiptAction::Remember, existing, false));
        }
        if state.live.len() + state.quarantined.len() >= self.inner.max_facts_per_scope {
            return Err(MemoryError::LimitReached);
        }
        self.lift_tombstone(&state, &body).await?;

        let now = Utc::now();
        let id = FactId::generate();
        let path = free_fact_path(&self.paths, &title, &id);
        let mut file = FactFile {
            path,
            fact: MemoryFact {
                id,
                revision: 1,
                title,
                hook,
                body,
                kind,
                provenance: Provenance::User,
                origin: origin_of(&actor),
                status: FactStatus::Confirmed,
                superseded_by: None,
                pinned: false,
                source_ref: source_ref.or_else(|| default_source_ref(&actor)),
                created_at: now,
                updated_at: now,
            },
            content_hash: None,
            extra: Mapping::new(),
        };
        file.refresh_hash();
        self.commit_fact(file, ReceiptAction::Remember, true).await
    }

    async fn correct(
        &self,
        actor: MemoryActor,
        id: FactId,
        expected_revision: u64,
        title: Option<String>,
        hook: Option<String>,
        body: String,
    ) -> Result<MemoryReceipt, MemoryError> {
        let (mut file, state) = find_live(self.state().await?, &id)?;
        check_revision(&file.fact, expected_revision)?;
        if let Some(title) = title {
            file.fact.title = single_line(&title);
        }
        if let Some(hook) = hook {
            file.fact.hook = single_line(&hook);
        }
        file.fact.body = fact_file::canonical_text(&body);
        guard(
            &file.fact.title,
            &file.fact.hook,
            &file.fact.body,
            self.inner.max_fact_body_chars,
        )?;
        self.lift_tombstone(&state, &file.fact.body).await?;

        file.fact.revision += 1;
        file.fact.updated_at = Utc::now();
        file.fact.origin = origin_of(&actor);
        file.refresh_hash();
        self.commit_fact(file, ReceiptAction::Correct, true).await
    }

    /// The content hash is left alone: it only covers text, and if the file was edited by hand
    /// before this, `reconcile` must still notice.
    async fn set_pinned(&self, id: FactId, pinned: bool) -> Result<MemoryReceipt, MemoryError> {
        let (mut file, _) = find_live(self.state().await?, &id)?;
        file.fact.pinned = pinned;
        self.commit_fact(file, ReceiptAction::SetPinned, false)
            .await
    }

    /// Commit point: the fact file.
    async fn commit_fact(
        &self,
        file: FactFile,
        action: ReceiptAction,
        reindex: bool,
    ) -> Result<MemoryReceipt, MemoryError> {
        if !step!(self.inner, Commit) {
            return Err(MemoryError::Unavailable);
        }
        let to_write = file.clone();
        let committed = blocking(move || write_fact(&to_write)).await?;
        let index_change = if reindex {
            IndexChange::Upsert(file.fact.clone())
        } else {
            IndexChange::Keep
        };
        let cleanup_pending = self.converge(index_change).await? | !committed.dir_synced;
        Ok(receipt(action, &file.fact, cleanup_pending))
    }

    /// Commit point: removing the fact file.
    async fn discard(&self, id: FactId, expected: u64) -> Result<MemoryReceipt, MemoryError> {
        let (file, _) = find_live(self.state().await?, &id)?;
        check_revision(&file.fact, expected)?;
        if !step!(self.inner, Commit) {
            return Err(MemoryError::Unavailable);
        }
        let path = file.path.clone();
        let committed = blocking(move || vault::commit_remove(&path)).await?;
        let cleanup_pending = self.converge(IndexChange::Remove(id)).await? | !committed.dir_synced;
        Ok(receipt(ReceiptAction::Discard, &file.fact, cleanup_pending))
    }

    /// Commit point: the tombstone. It already hides the fact from every read, which is why the
    /// file can be the last thing to go.
    async fn forget(&self, id: FactId, expected: u64) -> Result<MemoryReceipt, MemoryError> {
        let (file, state) = find_live(self.state().await?, &id)?;
        check_revision(&file.fact, expected)?;
        if !step!(self.inner, Commit) {
            return Err(MemoryError::Unavailable);
        }
        let mut tombstones = state.tombstones;
        tombstones.add(self.fingerprint(&file.fact.body), &Utc::now());
        let path = self.paths.tombstones.clone();
        let committed = blocking(move || tombstones.commit(&path)).await?;

        let mut cleanup_pending =
            self.converge(IndexChange::Remove(id)).await? | !committed.dir_synced;
        let removed = if step!(self.inner, Cleanup) {
            let path = file.path.clone();
            blocking(move || vault::remove_durable(&path)).await.is_ok()
        } else {
            false
        };
        cleanup_pending |= !removed;
        Ok(receipt(ReceiptAction::Forget, &file.fact, cleanup_pending))
    }

    /// Commit point: publishing nothing, so an open session stops seeing the vault at once.
    async fn purge(&self) -> Result<MemoryReceipt, MemoryError> {
        if !step!(self.inner, Commit) {
            return Err(MemoryError::Unavailable);
        }
        publish(self.entry, MemoryContext::default());
        let removed = if step!(self.inner, Cleanup) {
            let dir = self.paths.dir.clone();
            blocking(move || remove_scope_dir(&dir)).await.is_ok()
        } else {
            false
        };
        Ok(MemoryReceipt {
            action: ReceiptAction::PurgeScope,
            fact_id: None,
            title: None,
            revision: None,
            at: Utc::now(),
            cleanup_pending: !removed,
        })
    }

    /// Index, publish, `MEMORY.md`, in that order. Returns whether anything was left unfinished.
    /// `Err` only ever comes from a test's simulated crash: production steps cannot fail this call.
    async fn converge(&self, change: IndexChange) -> Result<bool, MemoryError> {
        let mut pending = false;

        let indexed = step!(self.inner, Index) && self.update_index(&change).await;
        if !indexed {
            pending = true;
            // No index beats a stale one: `recall` rebuilds what is missing.
            let path = self.paths.index.clone();
            if let Err(error) = blocking(move || index::discard(&path)).await {
                tracing::warn!(event = "memory_index_discard_failed", code = error.code());
            }
        }

        let derived: Option<Derived> = if step!(self.inner, Publish) {
            publish_from_disk(self.inner, self.dir, self.entry)
                .await
                .ok()
        } else {
            super::read::publish_degraded(self.entry);
            None
        };
        pending |= derived.is_none();

        let written = step!(self.inner, Summary)
            && match derived {
                Some(derived) => {
                    let paths = self.paths.clone();
                    blocking(move || write_summary_file(&paths, &derived.summary_file))
                        .await
                        .is_ok()
                }
                None => false,
            };
        pending |= !written;
        Ok(pending)
    }

    async fn update_index(&self, change: &IndexChange) -> bool {
        let result = match change {
            IndexChange::Upsert(fact) => index::upsert(&self.paths.index, fact).await,
            IndexChange::Remove(id) => index::remove(&self.paths.index, id.as_str()).await,
            IndexChange::Keep => return true,
        };
        if result.is_err() {
            tracing::warn!(event = "memory_index_update_failed");
        }
        result.is_ok()
    }
}

fn remove_scope_dir(dir: &std::path::Path) -> io::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    match dir.parent() {
        Some(parent) => std::fs::File::open(parent)?.sync_all(),
        None => Ok(()),
    }
}

fn origin_of(actor: &MemoryActor) -> FactOrigin {
    match actor {
        MemoryActor::Admin => FactOrigin::Admin,
        MemoryActor::VoiceUser { .. } => FactOrigin::Voice,
    }
}

fn default_source_ref(actor: &MemoryActor) -> Option<String> {
    match actor {
        MemoryActor::Admin => None,
        MemoryActor::VoiceUser { device_db_id, .. } => Some(format!("device:{device_db_id}")),
    }
}

fn receipt(action: ReceiptAction, fact: &MemoryFact, cleanup_pending: bool) -> MemoryReceipt {
    MemoryReceipt {
        action,
        fact_id: Some(fact.id.clone()),
        title: Some(fact.title.clone()),
        revision: Some(fact.revision),
        at: Utc::now(),
        cleanup_pending,
    }
}

/// Commit point: `profile.md`. It sits in every `MemoryContext`, so every open scope republishes.
/// Each scope's writer is taken in turn; Device writers never wait on this one, so this cannot
/// deadlock, and no scope can publish an older snapshot over this one.
async fn set_profile(
    inner: &Arc<Inner>,
    deployment_key: &str,
    body: String,
    expected_revision: u64,
) -> Result<MemoryReceipt, MemoryError> {
    let path = inner.vault.profile_path();
    let read_path = path.clone();
    let current = blocking(move || read_profile(&read_path)).await?;
    if current.profile.revision != expected_revision {
        return Err(MemoryError::RevisionConflict);
    }
    let body = fact_file::canonical_text(&body);
    guard("", "", &body, inner.max_fact_body_chars)?;

    let now = Utc::now();
    let revision = expected_revision + 1;
    let mut file: ProfileFile = current;
    file.profile.body = body;
    file.profile.revision = revision;
    file.profile.updated_at = Some(now);
    let committed =
        blocking(move || vault::commit_atomic(&path, render_profile(&file)?.as_bytes())).await?;

    let mut cleanup_pending = !committed.dir_synced;
    for (key, entry) in inner.registry.all() {
        let published = if key == deployment_key {
            publish_deployment(inner, &entry).await.is_ok()
        } else {
            let _writer = entry.writer.lock().await;
            publish_from_disk(inner, &key, &entry).await.is_ok()
        };
        cleanup_pending |= !published;
    }
    Ok(MemoryReceipt {
        action: ReceiptAction::SetProfile,
        fact_id: None,
        title: None,
        revision: Some(revision),
        at: now,
        cleanup_pending,
    })
}
