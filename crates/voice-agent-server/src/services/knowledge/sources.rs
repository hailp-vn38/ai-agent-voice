//! Canonical, immutable v1 Markdown Sources. No Job-owned artifact is promoted
//! implicitly; only authenticated Admin input may publish raw evidence.
use super::Error;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::{fs, io::AsyncWriteExt};
use uuid::Uuid;

const MAX_SOURCE_BYTES: usize = 256 * 1024;
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceInput {
    pub name: String,
    pub content: String,
    pub provenance: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub version: i64,
    pub name: String,
    pub provenance: String,
    pub sha256: String,
    pub size_bytes: i64,
    pub media_type: String,
}
pub struct SourceVault {
    root: PathBuf,
}
impl SourceVault {
    pub async fn open(knowledge_root: &Path) -> Result<Self, Error> {
        let root = knowledge_root.join("raw");
        fs::create_dir_all(&root).await?;
        let info = fs::symlink_metadata(&root).await?;
        if !info.is_dir() || info.file_type().is_symlink() {
            return Err(Error::InvalidPath);
        }
        Ok(Self { root })
    }
    pub async fn import_markdown(&self, input: SourceInput) -> Result<Source, Error> {
        if input.name.trim().is_empty()
            || input.name.len() > 255
            || input.name.chars().any(|c| c.is_control())
            || input.provenance.trim().is_empty()
            || input.provenance.len() > 1024
            || input.content.trim().is_empty()
            || input.content.len() > MAX_SOURCE_BYTES
        {
            return Err(Error::InvalidInput);
        }
        let id = format!("src_{}", Uuid::new_v4().simple());
        let sha256 = Sha256::digest(input.content.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let source = Source {
            id: id.clone(),
            version: 1,
            name: input.name,
            provenance: input.provenance,
            sha256,
            size_bytes: i64::try_from(input.content.len()).map_err(|_| Error::InvalidInput)?,
            media_type: "text/markdown".into(),
        };
        let source_root = self.root.join(&id);
        fs::create_dir(&source_root).await?;
        let staged = source_root.join("1.pending");
        fs::create_dir(&staged).await?;
        async fn write_new(path: &Path, data: &[u8]) -> Result<(), Error> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .await?;
            file.write_all(data).await?;
            file.sync_all().await?;
            Ok(())
        }
        write_new(&staged.join("original.md"), input.content.as_bytes()).await?;
        write_new(&staged.join("extracted.md"), input.content.as_bytes()).await?;
        let data = serde_json::to_vec_pretty(&source).map_err(|_| Error::InvalidInput)?;
        write_new(&staged.join("manifest.json"), &data).await?;
        fs::rename(&staged, source_root.join("1")).await?;
        super::vault::sync_dir(source_root.clone()).await?;
        super::vault::sync_dir(self.root.clone()).await?;
        Ok(source)
    }
    fn source_dir(&self, id: &str) -> Result<PathBuf, Error> {
        if !super::vault::valid_id(id) || !id.starts_with("src_") {
            return Err(Error::InvalidInput);
        }
        Ok(self.root.join(id).join("1"))
    }
    async fn manifest(&self, id: &str) -> Result<Source, Error> {
        let dir = self.source_dir(id)?;
        let parent = self.root.join(id);
        let parent_meta = fs::symlink_metadata(&parent)
            .await
            .map_err(|_| Error::NotFound)?;
        if parent_meta.file_type().is_symlink() || !parent_meta.is_dir() {
            return Err(Error::InvalidPath);
        }
        let meta = fs::symlink_metadata(&dir)
            .await
            .map_err(|_| Error::NotFound)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(Error::InvalidPath);
        }
        let manifest_path = dir.join("manifest.json");
        let meta = fs::symlink_metadata(&manifest_path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::NotFound
            } else {
                Error::Io(e)
            }
        })?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 4096 {
            return Err(Error::InvalidPath);
        }
        let manifest: Source = serde_json::from_slice(&fs::read(&manifest_path).await?)
            .map_err(|_| Error::InvalidInput)?;
        if manifest.id != id
            || manifest.version != 1
            || manifest.media_type != "text/markdown"
            || manifest.sha256.len() != 64
            || manifest.size_bytes < 0
            || manifest.size_bytes as usize > MAX_SOURCE_BYTES
        {
            return Err(Error::InvalidInput);
        }
        Ok(manifest)
    }
    pub async fn verify(&self, source_ref: &str) -> Result<Source, Error> {
        let (id, ver) = source_ref.split_once(':').ok_or(Error::InvalidInput)?;
        if ver != "1" {
            return Err(Error::InvalidInput);
        }
        let manifest = self.manifest(id).await?;
        let path = self.source_dir(id)?.join("original.md");
        let meta = fs::symlink_metadata(&path).await?;
        if !meta.is_file()
            || meta.file_type().is_symlink()
            || meta.len() as i64 != manifest.size_bytes
        {
            return Err(Error::InvalidPath);
        }
        let bytes = fs::read(&path).await?;
        let checksum = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if checksum != manifest.sha256 {
            return Err(Error::InvalidInput);
        }
        Ok(manifest)
    }
    pub async fn list(&self) -> Result<Vec<Source>, Error> {
        let mut dir = fs::read_dir(&self.root).await?;
        let mut records = Vec::new();
        while let Some(entry) = dir.next_entry().await? {
            let info = fs::symlink_metadata(entry.path()).await?;
            if info.file_type().is_symlink() {
                return Err(Error::InvalidPath);
            }
            if !info.is_dir() {
                continue;
            }
            let id = entry.file_name();
            let Some(id) = id.to_str() else {
                return Err(Error::InvalidInput);
            };
            // An interrupted unpublished staging directory must not become a Source.
            if !super::vault::valid_id(id) || !id.starts_with("src_") {
                continue;
            }
            match self.verify(&format!("{id}:1")).await {
                Ok(source) => records.push(source),
                Err(Error::NotFound) => {} // pending import, never published
                Err(error) => return Err(error),
            }
        }
        records.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(records)
    }
    pub async fn read_text(&self, id: &str) -> Result<String, Error> {
        self.verify(&format!("{id}:1")).await?;
        let bytes = fs::read(self.source_dir(id)?.join("extracted.md")).await?;
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(Error::InvalidInput);
        }
        let expected = self.manifest(id).await?;
        let checksum = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if checksum != expected.sha256 {
            return Err(Error::InvalidInput);
        }
        String::from_utf8(bytes).map_err(|_| Error::InvalidInput)
    }
}
#[cfg(test)]
mod tests;
