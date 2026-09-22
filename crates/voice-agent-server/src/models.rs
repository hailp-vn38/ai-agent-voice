//! Startup-only Model Preparation for pinned, provider-facing artifacts.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::config::{DeploymentConfig, ModelAcknowledgement};

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("cannot read model manifest: {0}")]
    Read(#[from] std::io::Error),
    #[error("invalid model manifest: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("model `{0}` is not declared by the manifest")]
    UnknownModel(String),
    #[error("deployment has not acknowledged {model}@{revision} ({license})")]
    MissingAcknowledgement {
        model: String,
        revision: String,
        license: String,
    },
    #[error("model `{model}` is not permitted by deployment profile `{profile}`")]
    LicenseDenied { model: String, profile: String },
    #[error("model artifact is missing: {0}")]
    MissingArtifact(String),
    #[error("offline Model Preparation cannot acquire missing or corrupt artifact `{0}`")]
    Offline(String),
    #[error("model artifact path is unsafe: {0}")]
    UnsafePath(String),
    #[error("unsupported model artifact transform: {0}")]
    UnsupportedTransform(String),
    #[error("model artifact acquisition failed: {0}")]
    Acquire(String),
    #[error("SHA-256 mismatch for {path}")]
    HashMismatch { path: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    model: Vec<Model>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub identity: String,
    pub adapter: String,
    pub source: String,
    pub revision: String,
    pub license: String,
    pub artifacts: Vec<Artifact>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub role: String,
    pub remote: String,
    pub install_path: PathBuf,
    pub source_sha256: String,
    pub sha256: String,
    #[serde(default = "identity_transform")]
    pub transform: String,
}

fn identity_transform() -> String {
    "identity".into()
}

/// Provider-facing, verified artifact paths addressed by manifest role.
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    identity: String,
    adapter: String,
    artifacts: Vec<(String, PathBuf)>,
}

impl ResolvedModel {
    pub fn artifact(&self, role: &str) -> Option<&Path> {
        self.artifacts
            .iter()
            .find(|(artifact_role, _)| artifact_role == role)
            .map(|(_, path)| path.as_path())
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }

    pub fn adapter(&self) -> &str {
        &self.adapter
    }
}

pub trait ModelAcquirer: Send + Sync {
    fn acquire(&self, remote: &str, destination: &Path) -> Result<(), ModelError>;
}

/// Default acquisition implementation used only during startup preparation.
pub struct HttpModelAcquirer;

impl ModelAcquirer for HttpModelAcquirer {
    fn acquire(&self, remote: &str, destination: &Path) -> Result<(), ModelError> {
        let response = reqwest::blocking::get(remote)
            .map_err(|error| ModelError::Acquire(error.to_string()))?
            .error_for_status()
            .map_err(|error| ModelError::Acquire(error.to_string()))?;
        let bytes = response
            .bytes()
            .map_err(|error| ModelError::Acquire(error.to_string()))?;
        fs::write(destination, bytes).map_err(ModelError::Read)
    }
}

pub struct ModelPreparationConfig {
    pub manifest_path: PathBuf,
    pub root: PathBuf,
    pub offline: bool,
}

/// Resolves only manifest-declared artifacts before providers are built or warmed.
pub struct ModelPreparation<A = HttpModelAcquirer> {
    config: ModelPreparationConfig,
    acquirer: A,
}

impl ModelPreparation<HttpModelAcquirer> {
    pub fn new(config: ModelPreparationConfig) -> Self {
        Self {
            config,
            acquirer: HttpModelAcquirer,
        }
    }
}

impl<A: ModelAcquirer> ModelPreparation<A> {
    pub fn with_acquirer(config: ModelPreparationConfig, acquirer: A) -> Self {
        Self { config, acquirer }
    }

    pub fn prepare(&self, identity: &str, adapter: &str) -> Result<ResolvedModel, ModelError> {
        let model = load_manifest(&self.config.manifest_path, identity, adapter)?;
        let artifacts = model
            .artifacts
            .iter()
            .map(|artifact| {
                self.prepare_artifact(artifact)
                    .map(|path| (artifact.role.clone(), path))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ResolvedModel {
            identity: model.identity,
            adapter: model.adapter,
            artifacts,
        })
    }

    fn prepare_artifact(&self, artifact: &Artifact) -> Result<PathBuf, ModelError> {
        validate_relative_path(&artifact.install_path)?;
        let installed = safe_install_path(&self.config.root, &artifact.install_path)?;
        if verifies(&installed, &artifact.sha256)? {
            return Ok(installed);
        }
        if self.config.offline {
            return Err(ModelError::Offline(
                artifact.install_path.display().to_string(),
            ));
        }
        let parent = installed
            .parent()
            .ok_or_else(|| ModelError::UnsafePath(artifact.install_path.display().to_string()))?;
        fs::create_dir_all(parent)?;
        let part = PathBuf::from(format!("{}.part", installed.display()));
        let transformed = PathBuf::from(format!("{}.transform", installed.display()));
        let _ = fs::remove_file(&part);
        let _ = fs::remove_file(&transformed);
        self.acquirer.acquire(&artifact.remote, &part)?;
        verify_path(&part, &artifact.source_sha256)?;
        transform(&part, &transformed, &artifact.transform)?;
        verify_path(&transformed, &artifact.sha256)?;
        fs::rename(&transformed, &installed)?;
        let _ = fs::remove_file(&part);
        Ok(installed)
    }
}

pub fn prepare(
    manifest_path: &Path,
    root: &Path,
    offline: bool,
    identity: &str,
    adapter: &str,
    deployment: &DeploymentConfig,
) -> Result<ResolvedModel, ModelError> {
    let model = load_manifest(manifest_path, identity, adapter)?;
    if deployment.profile == "commercial" && model.license.contains("NC") {
        return Err(ModelError::LicenseDenied {
            model: model.identity,
            profile: deployment.profile.clone(),
        });
    }
    require_acknowledgement(&model, &deployment.model_acknowledgements)?;
    ModelPreparation::new(ModelPreparationConfig {
        manifest_path: manifest_path.into(),
        root: root.into(),
        offline,
    })
    .prepare(identity, adapter)
}

fn load_manifest(manifest_path: &Path, identity: &str, adapter: &str) -> Result<Model, ModelError> {
    let manifest: Manifest = toml::from_str(&fs::read_to_string(manifest_path)?)?;
    let model = manifest
        .model
        .into_iter()
        .find(|model| model.identity == identity && model.adapter == adapter)
        .ok_or_else(|| ModelError::UnknownModel(identity.into()))?;
    Ok(model)
}

fn require_acknowledgement(
    model: &Model,
    acknowledgements: &[ModelAcknowledgement],
) -> Result<(), ModelError> {
    acknowledgements
        .iter()
        .any(|ack| {
            ack.model == model.identity
                && ack.revision == model.revision
                && ack.license == model.license
        })
        .then_some(())
        .ok_or_else(|| ModelError::MissingAcknowledgement {
            model: model.identity.clone(),
            revision: model.revision.clone(),
            license: model.license.clone(),
        })
}

fn verifies(path: &Path, expected: &str) -> Result<bool, ModelError> {
    match fs::read(path) {
        Ok(bytes) => Ok(hash(&bytes) == expected),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ModelError::Read(error)),
    }
}

fn verify_path(path: &Path, expected: &str) -> Result<(), ModelError> {
    let bytes = fs::read(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => ModelError::MissingArtifact(path.display().to_string()),
        _ => ModelError::Read(error),
    })?;
    (hash(&bytes) == expected)
        .then_some(())
        .ok_or_else(|| ModelError::HashMismatch {
            path: path.display().to_string(),
        })
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate_relative_path(path: &Path) -> Result<(), ModelError> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(ModelError::UnsafePath(path.display().to_string()));
    }
    Ok(())
}

fn safe_install_path(root: &Path, relative: &Path) -> Result<PathBuf, ModelError> {
    fs::create_dir_all(root)?;
    let canonical_root = fs::canonicalize(root)?;
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let mut current = root.to_path_buf();
    for component in parent.components() {
        let std::path::Component::Normal(part) = component else {
            continue;
        };
        current.push(part);
        if current.exists() {
            let resolved = fs::canonicalize(&current)?;
            if !resolved.starts_with(&canonical_root) {
                return Err(ModelError::UnsafePath(relative.display().to_string()));
            }
        } else {
            fs::create_dir(&current)?;
        }
    }
    let installed = root.join(relative);
    if installed.exists() && !fs::canonicalize(&installed)?.starts_with(&canonical_root) {
        return Err(ModelError::UnsafePath(relative.display().to_string()));
    }
    Ok(installed)
}

fn transform(source: &Path, destination: &Path, declared: &str) -> Result<(), ModelError> {
    let input = fs::read(source)?;
    let output = match declared {
        "identity" => input,
        "sentencepiece_tokens_v1" => sentencepiece_tokens(&input)?,
        declared if declared.starts_with("strip_prefix:") => {
            let length = declared["strip_prefix:".len()..]
                .parse::<usize>()
                .map_err(|_| ModelError::UnsupportedTransform(declared.into()))?;
            input
                .get(length..)
                .ok_or_else(|| ModelError::UnsupportedTransform(declared.into()))?
                .to_vec()
        }
        _ => return Err(ModelError::UnsupportedTransform(declared.into())),
    };
    fs::write(destination, output)?;
    Ok(())
}

fn sentencepiece_tokens(input: &[u8]) -> Result<Vec<u8>, ModelError> {
    let mut offset = 0;
    let mut tokens = Vec::new();
    while offset < input.len() {
        let tag = read_varint(input, &mut offset)?;
        let length = read_varint(input, &mut offset)? as usize;
        let end = offset
            .checked_add(length)
            .filter(|end| *end <= input.len())
            .ok_or_else(|| ModelError::UnsupportedTransform("sentencepiece_tokens_v1".into()))?;
        if tag == 0x0a {
            let nested = &input[offset..end];
            let mut piece_offset = 0;
            let piece_tag = read_varint(nested, &mut piece_offset)?;
            let piece_length = read_varint(nested, &mut piece_offset)? as usize;
            let piece_end = piece_offset
                .checked_add(piece_length)
                .filter(|piece_end| *piece_end <= nested.len())
                .ok_or_else(|| {
                    ModelError::UnsupportedTransform("sentencepiece_tokens_v1".into())
                })?;
            if piece_tag != 0x0a {
                return Err(ModelError::UnsupportedTransform(
                    "sentencepiece_tokens_v1".into(),
                ));
            }
            let piece = std::str::from_utf8(&nested[piece_offset..piece_end])
                .map_err(|_| ModelError::UnsupportedTransform("sentencepiece_tokens_v1".into()))?;
            tokens.push(format!("{piece} {}\n", tokens.len()));
        }
        offset = end;
    }
    (!tokens.is_empty())
        .then(|| tokens.concat().into_bytes())
        .ok_or_else(|| ModelError::UnsupportedTransform("sentencepiece_tokens_v1".into()))
}

fn read_varint(input: &[u8], offset: &mut usize) -> Result<u64, ModelError> {
    let mut value = 0_u64;
    for shift in (0..64).step_by(7) {
        let byte = *input
            .get(*offset)
            .ok_or_else(|| ModelError::UnsupportedTransform("sentencepiece_tokens_v1".into()))?;
        *offset += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(ModelError::UnsupportedTransform(
        "sentencepiece_tokens_v1".into(),
    ))
}
