use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    io::{Cursor, Read},
};
use zip::ZipArchive;

use super::PluginError;

const MAX_ZIP: usize = 20 * 1024 * 1024;
const MAX_UNPACKED: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub manifest_version: u32,
    pub plugin: PluginIdentity,
    pub runtime: Runtime,
    pub permissions: Permissions,
    pub agent: Agent,
    pub operations: BTreeMap<String, Operation>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginIdentity {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Runtime {
    pub kind: String,
    #[serde(default)]
    pub python: Option<String>,
    pub entrypoint: String,
    pub protocol: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    #[serde(default)]
    pub network_hosts: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub instructions: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub description: String,
    pub provides: Vec<String>,
    pub sources: Vec<String>,
    pub input_schema: String,
    pub output_schema: String,
    pub replay: String,
}

pub struct Package {
    pub manifest: Manifest,
    pub digest: String,
    pub files: Vec<(String, Vec<u8>)>,
    pub source_zip: Vec<u8>,
}

fn token(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
fn relative(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('/')
        && !name.contains('\\')
        && name.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.len() <= 80
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}
fn required<'a>(files: &'a [(String, Vec<u8>)], name: &str) -> Result<&'a [u8], PluginError> {
    files
        .iter()
        .find(|(path, _)| path == name)
        .map(|(_, data)| data.as_slice())
        .ok_or(PluginError::InvalidPackage)
}

pub fn validate(bytes: &[u8]) -> Result<Package, PluginError> {
    if bytes.is_empty() || bytes.len() > MAX_ZIP {
        return Err(PluginError::InvalidPackage);
    }
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|_| PluginError::InvalidPackage)?;
    if archive.is_empty() || archive.len() > 128 {
        return Err(PluginError::InvalidPackage);
    }
    let mut files = Vec::with_capacity(archive.len());
    let mut seen = HashSet::new();
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| PluginError::InvalidPackage)?;
        let name = entry.name().to_owned();
        let normalized = name.trim_end_matches('/');
        if !relative(normalized)
            || normalized.split('/').any(|part| part.starts_with('.'))
            || !seen.insert(normalized.to_ascii_lowercase())
            || entry.size() > MAX_UNPACKED
            || entry.compressed_size() == 0 && entry.size() > 0
            || entry.size() > entry.compressed_size().saturating_mul(1000)
            || entry
                .unix_mode()
                .is_some_and(|mode| !matches!(mode & 0o170000, 0 | 0o100000 | 0o040000))
        {
            return Err(PluginError::InvalidPackage);
        }
        if entry.is_dir() {
            if entry.size() != 0 {
                return Err(PluginError::InvalidPackage);
            }
            continue;
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o040000)
        {
            return Err(PluginError::InvalidPackage);
        }
        total = total
            .checked_add(entry.size())
            .ok_or(PluginError::InvalidPackage)?;
        if total > MAX_UNPACKED {
            return Err(PluginError::InvalidPackage);
        }
        let mut content = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut content)
            .map_err(|_| PluginError::InvalidPackage)?;
        if content.len() as u64 != entry.size() {
            return Err(PluginError::InvalidPackage);
        }
        files.push((name, content));
    }
    let manifest: Manifest = toml::from_str(
        std::str::from_utf8(required(&files, "plugin.toml")?)
            .map_err(|_| PluginError::InvalidPackage)?,
    )
    .map_err(|_| PluginError::InvalidPackage)?;
    let is_python = manifest.runtime.kind == "python_container";
    let is_rust = manifest.runtime.kind == "rust_container";
    if manifest.manifest_version != 1
        || !token(&manifest.plugin.id, 64)
        || !token(&manifest.plugin.version, 64)
        || manifest.plugin.name.is_empty()
        || manifest.plugin.name.len() > 128
        || manifest.plugin.description.len() > 1024
        || !(is_python || is_rust)
        || manifest.runtime.protocol != "jsonl-v1"
        || (is_python && (manifest.runtime.python.as_deref() != Some("3.12")
            || manifest.runtime.entrypoint != "src/main.py"))
        || (is_rust && (manifest.runtime.python.is_some()
            || manifest.runtime.entrypoint != "bin/plugin"))
        || manifest.operations.is_empty()
        || manifest.operations.len() > 16
        || manifest.permissions.network_hosts.len() > 16
        || manifest.permissions.network_hosts.iter().any(|h| !token(h, 128) || !h.contains('.'))
    {
        return Err(PluginError::InvalidPackage);
    }
    let instructions = required(&files, &manifest.agent.instructions)?;
    if !relative(&manifest.agent.instructions) || instructions.len() > 16 * 1024 {
        return Err(PluginError::InvalidPackage);
    }
    let executable = required(&files, &manifest.runtime.entrypoint)?;
    if is_rust {
        // Only a reviewed Linux ELF executable for the host architecture. Rust
        // packages are cross-compiled before upload; no untrusted build scripts
        // or dependency downloads run on the Plugin Store host.
        let machine: u16 = match std::env::consts::ARCH {
            "x86_64" => 62,
            "aarch64" => 183,
            _ => return Err(PluginError::InvalidPackage),
        };
        if executable.len() < 64
            || !executable.starts_with(b"\x7fELF")
            || executable[4] != 2 // ELFCLASS64
            || executable[5] != 1 // little endian
            || !matches!(u16::from_le_bytes([executable[16], executable[17]]), 2 | 3)
            || u16::from_le_bytes([executable[18], executable[19]]) != machine
        {
            return Err(PluginError::InvalidPackage);
        }
    } else {
        let project: toml::Value = toml::from_str(
            std::str::from_utf8(required(&files, "pyproject.toml")?)
                .map_err(|_| PluginError::InvalidPackage)?,
        ).map_err(|_| PluginError::InvalidPackage)?;
        let lock: toml::Value = toml::from_str(
            std::str::from_utf8(required(&files, "uv.lock")?)
                .map_err(|_| PluginError::InvalidPackage)?,
        ).map_err(|_| PluginError::InvalidPackage)?;
        if project.get("project")
                .and_then(|p| p.get("requires-python"))
                .and_then(toml::Value::as_str)
                .is_none_or(|v| !v.contains("3.12"))
            || lock.get("requires-python").and_then(toml::Value::as_str)
                .is_none_or(|v| !v.contains("3.12"))
            || project.get("project").and_then(|p| p.get("name"))
                .and_then(toml::Value::as_str) != Some(manifest.plugin.id.as_str())
            || project.get("project").and_then(|p| p.get("version"))
                .and_then(toml::Value::as_str) != Some(manifest.plugin.version.as_str())
            || !lock.get("package").and_then(toml::Value::as_array)
                .is_some_and(|packages| packages.iter().any(|package| {
                    package.get("name").and_then(toml::Value::as_str)
                        == Some(manifest.plugin.id.as_str())
                    && package.get("version").and_then(toml::Value::as_str)
                        == Some(manifest.plugin.version.as_str())
                }))
        {
            return Err(PluginError::InvalidPackage);
        }
        let lock_text = std::str::from_utf8(required(&files, "uv.lock")?)
            .map_err(|_| PluginError::InvalidPackage)?;
        if ["git+", "http://", "https://", "editable =", "path =", "sdist ="]
            .iter().any(|needle| lock_text.contains(needle))
        {
            return Err(PluginError::InvalidPackage);
        }
    }
    for (name, operation) in &manifest.operations {
        if !token(name, 64)
            || operation.replay != "safe_read"
            || operation.provides.len() > 16
            || operation.sources.len() > 16
            || operation.provides.iter().chain(&operation.sources)
                .any(|v| !token(v, 64))
            || !relative(&operation.input_schema)
            || !relative(&operation.output_schema)
        {
            return Err(PluginError::InvalidPackage);
        }
        for path in [&operation.input_schema, &operation.output_schema] {
            let schema: serde_json::Value = serde_json::from_slice(required(&files, path)?)
                .map_err(|_| PluginError::InvalidPackage)?;
            if !schema.is_object() {
                return Err(PluginError::InvalidPackage);
            }
        }
    }
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(Package {
        manifest,
        digest,
        files,
        source_zip: bytes.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::{ZipWriter, write::SimpleFileOptions};

    const FIXTURE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/fixture-research-1.0.0.zip"
    ));

    #[test]
    fn fixture_package_has_checked_manifest_and_lock() {
        let package = validate(FIXTURE).unwrap();
        assert_eq!(package.manifest.plugin.id, "fixture-research");
        assert_eq!(package.manifest.runtime.python.as_deref(), Some("3.12"));
        assert_eq!(package.manifest.operations.len(), 2);
    }

    #[test]
    fn zip_slip_and_duplicate_entries_are_rejected() {
        for paths in [vec!["../escape"], vec!["x", "X"], vec![".dockerignore"]] {
            let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
            for name in paths {
                writer
                    .start_file(name, SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(b"data").unwrap();
            }
            let bytes = writer.finish().unwrap().into_inner();
            assert!(matches!(validate(&bytes), Err(PluginError::InvalidPackage)));
        }
    }

    #[test]
    fn oversized_archive_is_rejected_before_extraction() {
        let bytes = vec![0u8; MAX_ZIP + 1];
        assert!(matches!(validate(&bytes), Err(PluginError::InvalidPackage)));
    }

    #[test]
    fn stale_lock_is_rejected() {
        let mut source = ZipArchive::new(Cursor::new(FIXTURE)).unwrap();
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for index in 0..source.len() {
            let mut entry = source.by_index(index).unwrap();
            let name = entry.name().to_owned();
            if entry.is_dir() {
                writer
                    .add_directory(name, SimpleFileOptions::default())
                    .unwrap();
            } else {
                writer
                    .start_file(&name, SimpleFileOptions::default())
                    .unwrap();
                if name == "uv.lock" {
                    writer
                        .write_all(b"version = 1\nrequires-python = \">=3.11\"\n")
                        .unwrap();
                } else {
                    std::io::copy(&mut entry, &mut writer).unwrap();
                }
            }
        }
        let bytes = writer.finish().unwrap().into_inner();
        assert!(matches!(validate(&bytes), Err(PluginError::InvalidPackage)));
    }
}
