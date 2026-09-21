//! Immutable model-artifact manifest validation at the startup boundary.

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
    pub path: PathBuf,
    pub sha256: String,
    #[serde(default)]
    pub derived_from: Option<PathBuf>,
}

impl Model {
    pub fn artifact(&self, name: &str) -> Option<&Artifact> {
        self.artifacts
            .iter()
            .find(|artifact| artifact.path.file_name().and_then(|x| x.to_str()) == Some(name))
    }
}

pub fn load_and_verify(
    manifest_path: &Path,
    identity: &str,
    adapter: &str,
    deployment: &DeploymentConfig,
) -> Result<Model, ModelError> {
    let manifest: Manifest = toml::from_str(&fs::read_to_string(manifest_path)?)?;
    let model = manifest
        .model
        .into_iter()
        .find(|model| model.identity == identity && model.adapter == adapter)
        .ok_or_else(|| ModelError::UnknownModel(identity.into()))?;
    if deployment.profile == "commercial" && model.license.contains("NC") {
        return Err(ModelError::LicenseDenied {
            model: model.identity,
            profile: deployment.profile.clone(),
        });
    }
    require_acknowledgement(&model, &deployment.model_acknowledgements)?;
    for artifact in &model.artifacts {
        verify_artifact(artifact)?;
    }
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

fn verify_artifact(artifact: &Artifact) -> Result<(), ModelError> {
    let bytes = fs::read(&artifact.path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => {
            ModelError::MissingArtifact(artifact.path.display().to_string())
        }
        _ => ModelError::Read(error),
    })?;
    let actual = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    (actual == artifact.sha256)
        .then_some(())
        .ok_or_else(|| ModelError::HashMismatch {
            path: artifact.path.display().to_string(),
        })
}
