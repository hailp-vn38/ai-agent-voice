//! Deep module managing the Job Skill catalog (`Built-in Job Skill` + `Custom Job Skill`)
//! stored in `<skills_dir>/<id>/SKILL.md`, using [`orchestrator_agent_rs::SkillDocument`]
//! as the single frontmatter/body codec and SHA-256 `ETag` preconditions for concurrency control.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use orchestrator_agent_rs::{
    AgentMode, SkillDocument, SkillDocumentError, valid_skill_id,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::app::AppState;

use super::agent_settings::{
    AGENT_SPECS, PersistedBackgroundSettings, save_persisted_settings,
    sync_orchestrator_from_state,
};
use super::workspace_seed::{BUILTIN_SKILLS, ensure_builtin_skills};

pub const SKILL_MAX_BYTES: usize = 64 * 1024;
const SKILL_FILE: &str = "SKILL.md";
const MAX_RELATED_FILES: usize = 20;
const MAX_RELATED_DEPTH: usize = 3;
const MAX_RELATED_SCAN: usize = 200;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSkillSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub is_builtin: bool,
    pub is_modified: bool,
    pub size_bytes: usize,
    pub etag: String,
    pub used_by_agents: Vec<String>,
    pub suggested_for_agents: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillDefaults {
    pub name: String,
    pub description: String,
    pub body: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSkillDetail {
    #[serde(flatten)]
    pub summary: JobSkillSummary,
    pub body: String,
    pub related_files: Vec<String>,
    pub default: Option<SkillDefaults>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SaveJobSkill {
    pub name: String,
    pub description: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Precondition {
    /// `If-None-Match: *` — create only if the skill does not yet exist.
    IfNoneMatchStar,
    /// `If-Match: "<etag>"` — update only if the current `SKILL.md` ETag matches.
    IfMatch(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum SkillCatalogError {
    InvalidId,
    InvalidArguments(&'static str),
    PayloadTooLarge,
    NotFound,
    AlreadyExists,
    RevisionConflict,
    BuiltinCannotBeDeleted,
    Io,
}

pub fn builtin_skill_raw(id: &str) -> Option<&'static str> {
    BUILTIN_SKILLS
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, v)| *v)
}

pub fn is_builtin_skill(id: &str) -> bool {
    builtin_skill_raw(id).is_some()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn builtin_defaults(id: &str) -> Option<SkillDefaults> {
    let raw = builtin_skill_raw(id)?;
    let doc = SkillDocument::parse(id, raw, SKILL_MAX_BYTES).ok()?;
    Some(SkillDefaults {
        name: doc.name,
        description: doc.description,
        body: doc.body,
    })
}

fn compute_agent_links(
    skill_id: &str,
    settings: &PersistedBackgroundSettings,
) -> (Vec<String>, Vec<String>) {
    let mut used_by = Vec::new();
    let mut suggested_for = Vec::new();
    for spec in AGENT_SPECS {
        if spec.mode == AgentMode::Primary {
            continue;
        }
        if spec.suggested_skills.contains(&skill_id) {
            suggested_for.push(spec.id.to_owned());
        }
        let is_used = match settings
            .agents
            .get(spec.id)
            .and_then(|o| o.allowed_skills.as_ref())
        {
            Some(list) => list.iter().any(|s| s == skill_id),
            None => spec.default_allowed_skills.contains(&skill_id),
        };
        if is_used {
            used_by.push(spec.id.to_owned());
        }
    }
    (used_by, suggested_for)
}

fn list_related_sync(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), String::new(), 0usize)];
    while let Some((path, rel, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in rd.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            let child_rel = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if ft.is_dir() {
                if depth + 1 < MAX_RELATED_DEPTH {
                    stack.push((entry.path(), child_rel, depth + 1));
                }
            } else if ft.is_file() && child_rel != SKILL_FILE {
                out.push(child_rel);
            }
            if out.len() >= MAX_RELATED_SCAN {
                break;
            }
        }
    }
    out.sort();
    out.truncate(MAX_RELATED_FILES);
    out
}

struct IndexedDiskSkill {
    doc: SkillDocument,
    dir: PathBuf,
    size_bytes: usize,
    etag: String,
}

fn read_one_skill_from_dir(root: &Path, id: &str) -> Result<IndexedDiskSkill, SkillCatalogError> {
    if !valid_skill_id(id) {
        return Err(SkillCatalogError::InvalidId);
    }
    let root_canon = std::fs::canonicalize(root).map_err(|_| SkillCatalogError::NotFound)?;
    let path = root_canon.join(id);
    let meta = std::fs::symlink_metadata(&path).map_err(|_| SkillCatalogError::NotFound)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(SkillCatalogError::NotFound);
    }
    let dir_canon = std::fs::canonicalize(&path).map_err(|_| SkillCatalogError::NotFound)?;
    if !dir_canon.starts_with(&root_canon) {
        return Err(SkillCatalogError::NotFound);
    }
    let file = dir_canon.join(SKILL_FILE);
    let file_meta = std::fs::symlink_metadata(&file).map_err(|_| SkillCatalogError::NotFound)?;
    if file_meta.file_type().is_symlink() || !file_meta.is_file() {
        return Err(SkillCatalogError::NotFound);
    }
    let text = std::fs::read_to_string(&file).map_err(|_| SkillCatalogError::Io)?;
    let size_bytes = text.len();
    let etag = sha256_hex(text.as_bytes());
    let doc = SkillDocument::parse(id, &text, SKILL_MAX_BYTES)
        .map_err(|_| SkillCatalogError::InvalidArguments("invalid_skill_document"))?;
    Ok(IndexedDiskSkill {
        doc,
        dir: dir_canon,
        size_bytes,
        etag,
    })
}

fn builtin_indexed(id: &str) -> Option<IndexedDiskSkill> {
    let raw = builtin_skill_raw(id)?;
    let doc = SkillDocument::parse(id, raw, SKILL_MAX_BYTES).ok()?;
    Some(IndexedDiskSkill {
        doc,
        dir: PathBuf::new(),
        size_bytes: raw.len(),
        etag: sha256_hex(raw.as_bytes()),
    })
}

fn scan_skills_dir(skills_dir: Option<&Path>) -> BTreeMap<String, IndexedDiskSkill> {
    let mut map = BTreeMap::new();
    let mut seen_names: BTreeSet<String> = BTreeSet::new();

    if let Some(root) = skills_dir
        && let Ok(rd) = std::fs::read_dir(root)
    {
        let mut names: Vec<String> = rd
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        names.sort();

        for id in names {
            if !valid_skill_id(&id) {
                continue;
            }
            if let Ok(indexed) = read_one_skill_from_dir(root, &id)
                && seen_names.insert(indexed.doc.name.clone())
            {
                map.insert(id, indexed);
            }
        }
    }

    for (id, _) in BUILTIN_SKILLS {
        if !map.contains_key(*id)
            && let Some(indexed) = builtin_indexed(id)
            && seen_names.insert(indexed.doc.name.clone())
        {
            map.insert((*id).to_owned(), indexed);
        }
    }

    map
}

/// Returns the sorted set of valid skill IDs currently available in `skills_dir`.
pub fn catalog_ids_from_dir(skills_dir: Option<&Path>) -> BTreeSet<String> {
    scan_skills_dir(skills_dir).into_keys().collect()
}

/// Returns the sorted set of valid skill IDs currently available in the server's `skills_dir`.
pub fn catalog_ids(state: &AppState) -> BTreeSet<String> {
    catalog_ids_from_dir(state.config.orchestrator.skills_dir.as_deref())
}

fn build_detail(
    indexed: IndexedDiskSkill,
    settings: &PersistedBackgroundSettings,
    include_related: bool,
) -> JobSkillDetail {
    let id = indexed.doc.id.clone();
    let default = builtin_defaults(&id);
    let is_builtin = default.is_some();
    let is_modified = default.as_ref().is_some_and(|def| {
        indexed.doc.name != def.name
            || indexed.doc.description != def.description
            || indexed.doc.body != def.body
    });
    let (used_by_agents, suggested_for_agents) = compute_agent_links(&id, settings);
    let related_files = if include_related && !indexed.dir.as_os_str().is_empty() {
        list_related_sync(&indexed.dir)
    } else {
        Vec::new()
    };

    JobSkillDetail {
        summary: JobSkillSummary {
            id,
            name: indexed.doc.name,
            description: indexed.doc.description,
            is_builtin,
            is_modified,
            size_bytes: indexed.size_bytes,
            etag: indexed.etag,
            used_by_agents,
            suggested_for_agents,
        },
        body: indexed.doc.body,
        related_files,
        default,
    }
}

fn current_settings(state: &AppState) -> PersistedBackgroundSettings {
    state
        .admin_task_control
        .lock()
        .expect("admin_task_control mutex poisoned")
        .background_settings
        .clone()
}

pub fn list(state: &AppState) -> Result<Vec<JobSkillSummary>, SkillCatalogError> {
    let settings = current_settings(state);
    let scanned = scan_skills_dir(state.config.orchestrator.skills_dir.as_deref());
    Ok(scanned
        .into_values()
        .map(|indexed| build_detail(indexed, &settings, false).summary)
        .collect())
}

pub fn get(state: &AppState, id: &str) -> Result<JobSkillDetail, SkillCatalogError> {
    if !valid_skill_id(id) {
        return Err(SkillCatalogError::InvalidId);
    }
    let Some(skills_dir) = state.config.orchestrator.skills_dir.as_deref() else {
        return Err(SkillCatalogError::NotFound);
    };
    let indexed = match read_one_skill_from_dir(skills_dir, id) {
        Ok(disk) => disk,
        Err(SkillCatalogError::NotFound) => {
            builtin_indexed(id).ok_or(SkillCatalogError::NotFound)?
        }
        Err(other) => return Err(other),
    };
    let settings = current_settings(state);
    Ok(build_detail(indexed, &settings, true))
}

fn map_doc_error(err: SkillDocumentError) -> SkillCatalogError {
    match err {
        SkillDocumentError::InvalidId => SkillCatalogError::InvalidId,
        SkillDocumentError::TooLarge { .. } => SkillCatalogError::PayloadTooLarge,
        SkillDocumentError::MissingField("name") | SkillDocumentError::InvalidField("name") => {
            SkillCatalogError::InvalidArguments("invalid_skill_name")
        }
        SkillDocumentError::MissingField("description")
        | SkillDocumentError::InvalidField("description") => {
            SkillCatalogError::InvalidArguments("invalid_skill_description")
        }
        SkillDocumentError::EmptyBody => SkillCatalogError::InvalidArguments("empty_skill_body"),
        SkillDocumentError::MissingFrontmatter
        | SkillDocumentError::MissingField(_)
        | SkillDocumentError::InvalidField(_) => {
            SkillCatalogError::InvalidArguments("invalid_arguments")
        }
    }
}

pub async fn save(
    state: &AppState,
    id: &str,
    input: SaveJobSkill,
    precondition: Precondition,
) -> Result<(JobSkillDetail, bool), SkillCatalogError> {
    if !valid_skill_id(id) {
        return Err(SkillCatalogError::InvalidId);
    }
    let Some(skills_dir) = state.config.orchestrator.skills_dir.as_deref() else {
        return Err(SkillCatalogError::Io);
    };

    let candidate = SkillDocument {
        id: id.to_owned(),
        name: input.name,
        description: input.description,
        body: input.body,
    };
    let rendered = candidate.render().map_err(map_doc_error)?;
    if rendered.len() > SKILL_MAX_BYTES {
        return Err(SkillCatalogError::PayloadTooLarge);
    }
    let verified = SkillDocument::parse(id, &rendered, SKILL_MAX_BYTES).map_err(map_doc_error)?;

    let _write_guard = state.skill_write_lock.lock().await;
    ensure_builtin_skills(skills_dir).map_err(|_| SkillCatalogError::Io)?;

    // Ensure no other skill in the catalog declares the same frontmatter `name`
    let existing_catalog = scan_skills_dir(Some(skills_dir));
    if existing_catalog
        .iter()
        .any(|(other_id, entry)| other_id != id && entry.doc.name == verified.name)
    {
        return Err(SkillCatalogError::InvalidArguments("duplicate_skill_name"));
    }

    let current_etag = read_one_skill_from_dir(skills_dir, id)
        .ok()
        .map(|s| s.etag);
    let created = match (&precondition, current_etag) {
        (Precondition::IfNoneMatchStar, Some(_)) => return Err(SkillCatalogError::AlreadyExists),
        (Precondition::IfMatch(_), None) => return Err(SkillCatalogError::NotFound),
        (Precondition::IfMatch(expected), Some(actual)) if *expected != actual => {
            return Err(SkillCatalogError::RevisionConflict);
        }
        (Precondition::IfNoneMatchStar, None) => true,
        (Precondition::IfMatch(_), Some(_)) => false,
    };

    let root_canon = std::fs::canonicalize(skills_dir).map_err(|_| SkillCatalogError::Io)?;
    let skill_dir = root_canon.join(id);
    if let Ok(meta) = std::fs::symlink_metadata(&skill_dir)
        && meta.file_type().is_symlink()
    {
        return Err(SkillCatalogError::InvalidArguments("invalid_arguments"));
    }
    std::fs::create_dir_all(&skill_dir).map_err(|_| SkillCatalogError::Io)?;
    let skill_file = skill_dir.join(SKILL_FILE);
    if let Ok(meta) = std::fs::symlink_metadata(&skill_file)
        && meta.file_type().is_symlink()
    {
        return Err(SkillCatalogError::InvalidArguments("invalid_arguments"));
    }
    let tmp_file = skill_dir.join(".SKILL.md.tmp");
    std::fs::write(&tmp_file, rendered.as_bytes()).map_err(|_| SkillCatalogError::Io)?;
    std::fs::rename(&tmp_file, &skill_file).map_err(|_| SkillCatalogError::Io)?;

    if let Some(orch) = state.orchestrator.as_ref()
        && let Err(err) = orch.reload_skills().await
    {
        tracing::warn!(error = %err, skill_id = %id, "failed to reload orchestrator skills after save");
    }
    if let Err(err) = sync_orchestrator_from_state(state).await {
        tracing::warn!(error = %err, skill_id = %id, "failed to reconfigure orchestrator after skill save");
    }

    let indexed = read_one_skill_from_dir(skills_dir, id)?;
    let settings = current_settings(state);
    Ok((build_detail(indexed, &settings, true), created))
}

pub async fn delete(
    state: &AppState,
    id: &str,
    expected_etag: &str,
) -> Result<(), SkillCatalogError> {
    if !valid_skill_id(id) {
        return Err(SkillCatalogError::InvalidId);
    }
    if is_builtin_skill(id) {
        return Err(SkillCatalogError::BuiltinCannotBeDeleted);
    }
    let Some(skills_dir) = state.config.orchestrator.skills_dir.as_deref() else {
        return Err(SkillCatalogError::NotFound);
    };

    let _write_guard = state.skill_write_lock.lock().await;
    ensure_builtin_skills(skills_dir).map_err(|_| SkillCatalogError::Io)?;
    let indexed = read_one_skill_from_dir(skills_dir, id)?;
    if indexed.etag != expected_etag {
        return Err(SkillCatalogError::RevisionConflict);
    }

    std::fs::remove_dir_all(&indexed.dir).map_err(|_| SkillCatalogError::Io)?;

    // Prune deleted custom skill ID from persisted agent settings if referenced
    let maybe_updated_settings = {
        let mut guard = state
            .admin_task_control
            .lock()
            .expect("admin_task_control mutex poisoned");
        let mut changed = false;
        for ov in guard.background_settings.agents.values_mut() {
            if let Some(skills) = ov.allowed_skills.as_mut() {
                let before = skills.len();
                skills.retain(|s| s != id);
                if skills.len() != before {
                    changed = true;
                }
            }
        }
        if changed {
            Some(guard.background_settings.clone())
        } else {
            None
        }
    };
    if let Some(updated_settings) = maybe_updated_settings
        && let Err(err) =
            save_persisted_settings(&state.config.orchestrator.store_dir, &updated_settings)
    {
        tracing::warn!(error = %err, skill_id = %id, "failed to persist pruned agent_settings.json after skill delete");
    }

    if let Some(orch) = state.orchestrator.as_ref()
        && let Err(err) = orch.reload_skills().await
    {
        tracing::warn!(error = %err, skill_id = %id, "failed to reload orchestrator skills after delete");
    }
    if let Err(err) = sync_orchestrator_from_state(state).await {
        tracing::warn!(error = %err, skill_id = %id, "failed to reconfigure orchestrator after skill delete");
    }

    Ok(())
}
