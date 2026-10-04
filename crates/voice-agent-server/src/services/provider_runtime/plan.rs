use serde::Serialize;

use super::ResourceKey;

/// Adapter-owned description of the native state that may be shared. Desired provider identity,
/// quota, and per-session selection deliberately do not belong here.
#[derive(Clone, Debug)]
pub struct LocalRuntimePlan {
    adapter: &'static str,
    model_identity: String,
    adapter_spec: serde_json::Value,
    execution: LocalExecutionRequirements,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalExecutionRequirements {
    Onnx,
    OnnxWithKokoroG2p,
}

impl LocalRuntimePlan {
    pub fn onnx(
        adapter: &'static str,
        model_identity: impl Into<String>,
        adapter_spec: serde_json::Value,
    ) -> Self {
        Self {
            adapter,
            model_identity: model_identity.into(),
            adapter_spec,
            execution: LocalExecutionRequirements::Onnx,
        }
    }

    pub fn onnx_with_kokoro_g2p(
        adapter: &'static str,
        model_identity: impl Into<String>,
        adapter_spec: serde_json::Value,
    ) -> Self {
        Self {
            adapter,
            model_identity: model_identity.into(),
            adapter_spec,
            execution: LocalExecutionRequirements::OnnxWithKokoroG2p,
        }
    }

    pub fn model_identity(&self) -> &str {
        &self.model_identity
    }

    pub fn requires_kokoro_g2p(&self) -> bool {
        self.execution == LocalExecutionRequirements::OnnxWithKokoroG2p
    }

    pub fn resource_key(
        &self,
        artifact_fingerprint: String,
        onnx_execution_fingerprint: [u8; 32],
        auxiliary_execution_fingerprint: Option<[u8; 32]>,
        execution_threads: usize,
        capacity: usize,
    ) -> Result<ResourceKey, serde_json::Error> {
        let identity = PhysicalResourceIdentity {
            adapter: self.adapter,
            artifact_fingerprint,
            onnx_execution_fingerprint,
            auxiliary_execution_fingerprint,
            execution_threads,
            capacity,
            adapter_spec: &self.adapter_spec,
        };
        let bytes = serde_json::to_vec(&identity)?;
        use sha2::{Digest, Sha256};
        Ok(ResourceKey(Sha256::digest(bytes).into()))
    }
}

/// Canonical envelope serialized once for every local resource identity. Adapter-specific state
/// stays in `adapter_spec`; common execution state is never duplicated there.
#[derive(Serialize)]
struct PhysicalResourceIdentity<'a> {
    adapter: &'static str,
    artifact_fingerprint: String,
    onnx_execution_fingerprint: [u8; 32],
    auxiliary_execution_fingerprint: Option<[u8; 32]>,
    execution_threads: usize,
    capacity: usize,
    adapter_spec: &'a serde_json::Value,
}
