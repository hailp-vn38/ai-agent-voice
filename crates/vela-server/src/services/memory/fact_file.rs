//! A fact on disk: YAML frontmatter plus Markdown body. Unknown frontmatter fields survive a rewrite.

use super::types::{
    FactId, FactKind, FactOrigin, FactStatus, MemoryFact, MemoryProfile, Provenance, format_time,
};
use chrono::{DateTime, Utc};
use serde_yaml::{Mapping, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

/// Bad YAML or a missing required field. Callers quarantine the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FactFileError;

#[derive(Clone, Debug)]
pub(super) struct FactFile {
    pub(super) path: PathBuf,
    pub(super) fact: MemoryFact,
    /// As found on disk. `None` or a mismatch means the file changed outside the system.
    pub(super) content_hash: Option<String>,
    pub(super) extra: Mapping,
}

impl FactFile {
    pub(super) fn is_owner_edited(&self) -> bool {
        self.content_hash.as_deref() != Some(content_hash(&self.fact).as_str())
    }

    pub(super) fn refresh_hash(&mut self) {
        self.content_hash = Some(content_hash(&self.fact));
    }
}

/// Line endings and surrounding blank lines are not content: editors change them freely.
pub(super) fn canonical_text(text: &str) -> String {
    text.replace("\r\n", "\n").trim().to_string()
}

pub(super) fn content_hash(fact: &MemoryFact) -> String {
    let mut hasher = Sha256::new();
    for field in [
        canonical_text(&fact.title),
        canonical_text(&fact.hook),
        fact.kind.as_str().to_string(),
        canonical_text(&fact.body),
    ] {
        hasher.update(field.as_bytes());
        hasher.update([0_u8]);
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn take_string(map: &mut Mapping, key: &str) -> Result<String, FactFileError> {
    match map.remove(key) {
        Some(Value::String(value)) => Ok(value),
        _ => Err(FactFileError),
    }
}

fn take_optional_string(map: &mut Mapping, key: &str) -> Result<Option<String>, FactFileError> {
    match map.remove(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        _ => Err(FactFileError),
    }
}

fn take_time(map: &mut Mapping, key: &str) -> Result<DateTime<Utc>, FactFileError> {
    DateTime::parse_from_rfc3339(&take_string(map, key)?)
        .map(|time| time.with_timezone(&Utc))
        .map_err(|_| FactFileError)
}

fn take_u64(map: &mut Mapping, key: &str) -> Result<u64, FactFileError> {
    map.remove(key)
        .and_then(|value| value.as_u64())
        .ok_or(FactFileError)
}

fn split(text: &str) -> Result<(Mapping, String), FactFileError> {
    let parsed = llm_wiki::parse_frontmatter(text);
    let yaml = parsed.raw_yaml.ok_or(FactFileError)?;
    let map = serde_yaml::from_str(&yaml).map_err(|_| FactFileError)?;
    Ok((map, canonical_text(&parsed.body)))
}

pub(super) fn parse(path: &Path, text: &str) -> Result<FactFile, FactFileError> {
    let (mut map, body) = split(text)?;
    let fact = MemoryFact {
        id: FactId::parse(&take_string(&mut map, "id")?).ok_or(FactFileError)?,
        title: take_string(&mut map, "title")?,
        hook: take_string(&mut map, "hook")?,
        kind: FactKind::parse(&take_string(&mut map, "kind")?).ok_or(FactFileError)?,
        status: FactStatus::parse(&take_string(&mut map, "status")?).ok_or(FactFileError)?,
        revision: take_u64(&mut map, "revision")?,
        created_at: take_time(&mut map, "created_at")?,
        updated_at: take_time(&mut map, "updated_at")?,
        provenance: match take_optional_string(&mut map, "provenance")? {
            Some(value) => Provenance::parse(&value).ok_or(FactFileError)?,
            None => Provenance::User,
        },
        origin: match take_optional_string(&mut map, "origin")? {
            Some(value) => FactOrigin::parse(&value).ok_or(FactFileError)?,
            None => FactOrigin::OwnerEdited,
        },
        superseded_by: take_optional_string(&mut map, "superseded_by")?,
        pinned: match map.remove("pinned") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => value,
            Some(_) => return Err(FactFileError),
        },
        source_ref: take_optional_string(&mut map, "source_ref")?,
        body,
    };
    let content_hash = take_optional_string(&mut map, "content_hash")?;
    Ok(FactFile {
        path: path.to_path_buf(),
        fact,
        content_hash,
        extra: map,
    })
}

fn optional(value: &Option<String>) -> Value {
    value.clone().map_or(Value::Null, Value::String)
}

fn document(front: &Mapping, body: &str) -> io::Result<String> {
    let yaml = serde_yaml::to_string(front).map_err(|_| io::ErrorKind::InvalidData)?;
    Ok(format!("---\n{yaml}---\n{body}\n"))
}

pub(super) fn render(file: &FactFile) -> io::Result<String> {
    let fact = &file.fact;
    let mut front = Mapping::new();
    let mut put = |key: &str, value: Value| {
        front.insert(Value::String(key.to_string()), value);
    };
    put("id", fact.id.as_str().into());
    put("title", fact.title.as_str().into());
    put("hook", fact.hook.as_str().into());
    put("kind", fact.kind.as_str().into());
    put("provenance", fact.provenance.as_str().into());
    put("origin", fact.origin.as_str().into());
    put("status", fact.status.as_str().into());
    put("revision", fact.revision.into());
    put("superseded_by", optional(&fact.superseded_by));
    put("pinned", fact.pinned.into());
    put("source_ref", optional(&fact.source_ref));
    put("created_at", format_time(&fact.created_at).into());
    put("updated_at", format_time(&fact.updated_at).into());
    put("content_hash", optional(&file.content_hash));
    for (key, value) in &file.extra {
        front.insert(key.clone(), value.clone());
    }
    document(&front, &fact.body)
}

/// Every `*.md` in `facts/`, valid or not. `facts/` being absent just means an empty vault.
#[derive(Debug, Default)]
pub(super) struct Scan {
    pub(super) facts: Vec<FactFile>,
    pub(super) quarantined: Vec<String>,
}

pub(super) fn scan(facts_dir: &Path) -> io::Result<Scan> {
    let entries = match fs::read_dir(facts_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Scan::default()),
        Err(error) => return Err(error),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let visible = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| !name.starts_with('.') && name.ends_with(".md"));
        if visible && path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();

    let mut scan = Scan::default();
    let mut seen = HashSet::new();
    for path in paths {
        let parsed = String::from_utf8(fs::read(&path)?)
            .map_err(|_| FactFileError)
            .and_then(|text| parse(&path, &text))
            // A copied file repeats its id; only the first one owns it.
            .and_then(|file| {
                if seen.insert(file.fact.id.clone()) {
                    Ok(file)
                } else {
                    Err(FactFileError)
                }
            });
        match parsed {
            Ok(file) => scan.facts.push(file),
            Err(FactFileError) => scan.quarantined.push(
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
        }
    }
    Ok(scan)
}

/// `profile.md`. Plain text without frontmatter is accepted as revision 0, since it may be hand-made.
pub(super) struct ProfileFile {
    pub(super) profile: MemoryProfile,
    pub(super) extra: Mapping,
}

impl ProfileFile {
    pub(super) fn empty() -> Self {
        Self {
            profile: MemoryProfile {
                body: String::new(),
                revision: 0,
                updated_at: None,
            },
            extra: Mapping::new(),
        }
    }
}

pub(super) fn read_profile(path: &Path) -> io::Result<ProfileFile> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(ProfileFile::empty()),
        Err(error) => return Err(error),
    };
    if llm_wiki::parse_frontmatter(&text).raw_yaml.is_none() {
        let mut file = ProfileFile::empty();
        file.profile.body = canonical_text(&text);
        return Ok(file);
    }
    let parse = || -> Result<ProfileFile, FactFileError> {
        let (mut map, body) = split(&text)?;
        let revision = take_u64(&mut map, "revision")?;
        let updated_at = take_time(&mut map, "updated_at")?;
        Ok(ProfileFile {
            profile: MemoryProfile {
                body,
                revision,
                updated_at: Some(updated_at),
            },
            extra: map,
        })
    };
    parse().map_err(|_| io::ErrorKind::InvalidData.into())
}

pub(super) fn render_profile(file: &ProfileFile) -> io::Result<String> {
    let mut front = Mapping::new();
    front.insert("revision".into(), file.profile.revision.into());
    front.insert(
        "updated_at".into(),
        file.profile
            .updated_at
            .as_ref()
            .map_or(Value::Null, |time| format_time(time).into()),
    );
    for (key, value) in &file.extra {
        front.insert(key.clone(), value.clone());
    }
    document(&front, &file.profile.body)
}
