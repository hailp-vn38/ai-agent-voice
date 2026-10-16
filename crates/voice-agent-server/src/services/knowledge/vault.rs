//! Canonical Markdown vault shared internally by Knowledge and Memory.
//! The caller owns domain policy. This module only owns validated files,
//! revision CAS, immutable history, and same-directory atomic replacement.
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, path::{Path, PathBuf}, time::UNIX_EPOCH};
use tokio::{fs, io::AsyncWriteExt, sync::Mutex};
use uuid::Uuid;
use super::Error;

const MAX_PAGE_BYTES: usize = 512 * 1024;
const MAX_HEADER_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct Record {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub revision: i64,
    pub status: String,
    pub scope: Option<String>,
    pub confirmed: bool,
    pub source_ref: Option<String>,
}

#[derive(Clone)]
pub struct Scanned {
    pub record: Record,
    pub content: String,
    pub relative_path: String,
    pub content_sha256: String,
    pub mtime_ns: i64,
    pub size_bytes: i64,
}

pub struct MarkdownVault {
    root: PathBuf,
    folder: &'static str,
    pub(crate) writer: Mutex<()>,
}
impl MarkdownVault {
    pub async fn new(root: PathBuf, folder: &'static str) -> Result<Self, Error> {
        fs::create_dir_all(root.join(folder)).await?;
        fs::create_dir_all(root.join("revisions")).await?;
        for directory in [&root, &root.join(folder), &root.join("revisions")] {
            let stat = fs::symlink_metadata(directory).await?;
            if stat.file_type().is_symlink() || !stat.is_dir() { return Err(Error::InvalidPath); }
        }
        Ok(Self { root, folder, writer: Mutex::new(()) })
    }
    pub fn root(&self) -> &Path { &self.root }
    pub fn item_path(&self, id: &str) -> Result<PathBuf, Error> {
        if !valid_id(id) { return Err(Error::InvalidInput); }
        Ok(self.root.join(self.folder).join(format!("{id}.md")))
    }
    pub async fn get(&self, id: &str) -> Result<Record, Error> {
        let path = self.item_path(id)?;
        let meta = fs::symlink_metadata(&path).await.map_err(map_missing)?;
        if !meta.is_file() || meta.file_type().is_symlink() { return Err(Error::InvalidPath); }
        let bytes = fs::read(&path).await?;
        if bytes.len() > MAX_PAGE_BYTES { return Err(Error::InvalidInput); }
        let content = std::str::from_utf8(&bytes).map_err(|_| Error::InvalidInput)?;
        let record = parse(content)?;
        if record.id != id { return Err(Error::InvalidInput); }
        Ok(record)
    }
    /// The caller MUST hold writer while comparing and publishing revisions.
    pub async fn commit(&self, record: &Record, previous: Option<&Record>) -> Result<(), Error> {
        let path = self.item_path(&record.id)?;
        if record.body.len() > MAX_PAGE_BYTES || record.title.len() > 512 { return Err(Error::InvalidInput); }
        let text = encode(record)?;
        if text.len() > MAX_PAGE_BYTES { return Err(Error::InvalidInput); }
        if let Some(previous) = previous {
            let old_path = self.item_path(&previous.id)?;
            let old = fs::read(&old_path).await?;
            let revision_dir = self.root.join("revisions").join(&record.id);
            fs::create_dir_all(&revision_dir).await?;
            let history = revision_dir.join(format!("{}.md", previous.revision));
            // A replay must never destroy a previously retained revision.
            match fs::OpenOptions::new().write(true).create_new(true).open(&history).await {
                Ok(mut f) => { f.write_all(&old).await?; f.sync_all().await?; }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if fs::read(&history).await? != old { return Err(Error::RevisionConflict); }
                }
                Err(err) => return Err(Error::Io(err)),
            }
        }
        let tmp = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
        let result = async {
            let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp).await?;
            f.write_all(text.as_bytes()).await?;
            f.sync_all().await?;
            drop(f);
            fs::rename(&tmp, &path).await?;
            Ok::<(), std::io::Error>(())
        }.await;
        if result.is_err() { let _ = fs::remove_file(&tmp).await; }
        result?;
        // Atomic rename requires a directory fsync for crash-durable publication.
        let parent = path.parent().ok_or(Error::InvalidPath)?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    }

    /// Scan only Core-managed single-level Markdown files. Never follow symlinks.
    /// An incomplete scan must return an error; index must not mass-prune.
    pub async fn scan(&self) -> Result<Vec<Scanned>, Error> {
        let folder = self.root.join(self.folder);
        let stat = fs::symlink_metadata(&folder).await.map_err(map_missing)?;
        if !stat.is_dir() || stat.file_type().is_symlink() { return Err(Error::Unavailable); }
        let mut dir = fs::read_dir(&folder).await?;
        let mut seen = HashSet::new();
        let mut documents = Vec::new();
        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).await?;
            if meta.file_type().is_symlink() { return Err(Error::InvalidPath); }
            if path.extension().and_then(|e| e.to_str()) != Some("md") { continue; }
            if !meta.is_file() || meta.len() as usize > MAX_PAGE_BYTES { return Err(Error::InvalidInput); }
            let bytes = fs::read(&path).await?;
            let content = String::from_utf8(bytes).map_err(|_| Error::InvalidInput)?;
            let record = parse(&content)?;
            if path.file_stem().and_then(|s| s.to_str()) != Some(record.id.as_str())
                || !seen.insert(record.id.clone()) { return Err(Error::InvalidInput); }
            let mtime_ns = meta.modified()?.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
            let mtime_ns = i64::try_from(mtime_ns).map_err(|_| Error::InvalidInput)?;
            let size_bytes = i64::try_from(content.len()).map_err(|_| Error::InvalidInput)?;
            let hash = format!("{:x}", Sha256::digest(content.as_bytes()));
            documents.push(Scanned {
                relative_path: format!("{}/{}.md", self.folder, record.id),
                record, content, content_sha256: hash, mtime_ns, size_bytes
            });
        }
        documents.sort_by(|a,b| a.record.id.cmp(&b.record.id));
        Ok(documents)
    }
}
fn map_missing(err: std::io::Error) -> Error {
    if err.kind() == std::io::ErrorKind::NotFound { Error::NotFound }
    else { Error::Io(err) }
}
pub fn valid_id(id: &str) -> bool {
    (4..=100).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn scalar(value: &str) -> Result<String, Error> {
    if value.starts_with('"') {
        serde_json::from_str::<String>(value).map_err(|_| Error::InvalidInput)
    } else if value.trim() == value && !value.chars().any(|c| matches!(c,'#'|'['|']'|'{'|'}'|'\r'|'\n')) {
        Ok(value.to_owned())
    } else { Err(Error::InvalidInput) }
}
fn string(value: &str) -> Result<String, Error> {
    serde_json::to_string(value).map_err(|_| Error::InvalidInput)
}
pub fn encode(rec: &Record) -> Result<String, Error> {
    if !valid_id(&rec.id) || rec.title.trim().is_empty() || rec.revision < 1
        || !matches!(rec.kind.as_str(), "wiki" | "memory") { return Err(Error::InvalidInput); }
    let mut s = format!("---\nid: {}\nkind: {}\ntitle: {}\nrevision: {}\nstatus: {}\n",
        rec.id, rec.kind, string(&rec.title)?, rec.revision, rec.status);
    if let Some(scope) = &rec.scope { s.push_str(&format!("scope: {}\n", string(scope)?)); }
    s.push_str(if rec.confirmed { "confirmed: true\n" } else { "confirmed: false\n" });
    if let Some(source) = &rec.source_ref { s.push_str(&format!("source_ref: {}\n", string(source)?)); }
    s.push_str("---\n");
    s.push_str(&rec.body);
    Ok(s)
}
pub fn parse(text: &str) -> Result<Record, Error> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.len() > MAX_PAGE_BYTES { return Err(Error::InvalidInput); }
    // Accept CRLF from imported Markdown while keeping canonically written LF.
    let normalized = text.replace("\r\n","\n");
    let (header, body) = normalized.strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .ok_or(Error::InvalidInput)?;
    if header.len() > MAX_HEADER_BYTES { return Err(Error::InvalidInput); }
    let mut attrs = std::collections::BTreeMap::<&str,String>::new();
    for line in header.lines() {
        let (key,value) = line.trim_end_matches('\r').split_once(": ").ok_or(Error::InvalidInput)?;
        if !matches!(key,"id"|"kind"|"title"|"revision"|"status"|"scope"|"confirmed"|"source_ref")
            || attrs.insert(key,scalar(value)?).is_some() { return Err(Error::InvalidInput); }
    }
    let required = |name| attrs.get(name).cloned().ok_or(Error::InvalidInput);
    let id = required("id")?;
    let kind = required("kind")?;
    let title = required("title")?;
    let revision = required("revision")?.parse().map_err(|_| Error::InvalidInput)?;
    let status = required("status")?;
    let confirmed = required("confirmed")?.parse().map_err(|_| Error::InvalidInput)?;
    let rec = Record { id,kind,title,revision,status,confirmed,
        scope:attrs.get("scope").cloned(),
        source_ref:attrs.get("source_ref").cloned(),body:body.to_owned() };
    if !valid_id(&rec.id) { return Err(Error::InvalidInput); }
    Ok(rec)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn round_trip_and_no_path_escape() {
        let p = Record { id:"wiki_1".into(),kind:"wiki".into(),title:"Vietnamese: Trí nhớ".into(),
            body:"# Welcome\nText".into(),revision:1,status:"draft".into(),scope:None,
            confirmed:false,source_ref:Some("src_1:1".into()) };
        let encoded=encode(&p).unwrap();
        let decoded=parse(&encoded).unwrap();
        assert_eq!(p.id,decoded.id);
        assert_eq!(p.title,decoded.title);
        assert_eq!(p.source_ref,decoded.source_ref);
        assert!(!valid_id("../secret"));
        assert!(parse("---\nid: x\n---\n").is_err());
    }
}
