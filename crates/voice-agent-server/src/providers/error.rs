use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AsrError {
    #[error("ASR provider failed: {0}")]
    Failed(String),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum VadError {
    #[error("VAD provider failed: {0}")]
    Failed(String),
}

#[derive(Debug, Error)]
pub enum ProviderLoadError {
    #[error("provider configuration is invalid: {0}")]
    Configuration(String),
    #[error("unsupported {kind} adapter `{adapter}`")]
    UnsupportedAdapter { kind: &'static str, adapter: String },
    #[error("provider model asset is missing: {0}")]
    MissingArtifact(String),
    #[error("cannot initialize local {0} provider")]
    Initialize(&'static str),
    #[error("provider initialization failed: {0}")]
    Provider(String),
}
