use serde::Serialize;

use super::ResourceKey;

/// Backing native topology one materialization owns. This is deliberately independent of
/// application-level concurrency: raising `workers.tts.max_workers` must not multiply resident
/// ONNX sessions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalCapacity {
    /// Backing topology follows the generic logical worker capacity. Correct for adapters that
    /// hold no resident per-worker native state.
    FollowsLogicalCapacity,
    /// Fixed replica count owned by the physical runtime, chosen by the adapter planner.
    Replicas(usize),
}

impl PhysicalCapacity {
    /// Never returns zero, so a fixed replica count of zero becomes a single replica rather than an
    /// unusable pool.
    pub fn resolve(self, logical: usize) -> usize {
        match self {
            Self::FollowsLogicalCapacity => logical,
            Self::Replicas(replicas) => replicas.max(1),
        }
    }
}

/// Adapter-owned description of the native state that may be shared. Desired provider identity,
/// quota, and per-session selection deliberately do not belong here.
#[derive(Clone, Debug)]
pub struct LocalRuntimePlan {
    adapter: &'static str,
    model_identity: String,
    adapter_spec: serde_json::Value,
    execution: LocalExecutionRequirements,
    physical_capacity: PhysicalCapacity,
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
        Self::with_capacity(
            adapter,
            model_identity,
            adapter_spec,
            LocalExecutionRequirements::Onnx,
            PhysicalCapacity::FollowsLogicalCapacity,
        )
    }

    pub fn onnx_with_kokoro_g2p(
        adapter: &'static str,
        model_identity: impl Into<String>,
        adapter_spec: serde_json::Value,
    ) -> Self {
        Self::with_capacity(
            adapter,
            model_identity,
            adapter_spec,
            LocalExecutionRequirements::OnnxWithKokoroG2p,
            PhysicalCapacity::FollowsLogicalCapacity,
        )
    }

    /// Declares a plan whose backing native topology is fixed rather than derived from the
    /// generic worker capacity.
    pub fn onnx_with_replicas(
        adapter: &'static str,
        model_identity: impl Into<String>,
        adapter_spec: serde_json::Value,
        replicas: usize,
    ) -> Self {
        Self::with_capacity(
            adapter,
            model_identity,
            adapter_spec,
            LocalExecutionRequirements::Onnx,
            PhysicalCapacity::Replicas(replicas),
        )
    }

    fn with_capacity(
        adapter: &'static str,
        model_identity: impl Into<String>,
        adapter_spec: serde_json::Value,
        execution: LocalExecutionRequirements,
        physical_capacity: PhysicalCapacity,
    ) -> Self {
        Self {
            adapter,
            model_identity: model_identity.into(),
            adapter_spec,
            execution,
            physical_capacity,
        }
    }

    pub fn model_identity(&self) -> &str {
        &self.model_identity
    }

    pub fn requires_kokoro_g2p(&self) -> bool {
        self.execution == LocalExecutionRequirements::OnnxWithKokoroG2p
    }

    pub fn physical_capacity(&self) -> PhysicalCapacity {
        self.physical_capacity
    }

    pub fn resource_key(
        &self,
        artifact_fingerprint: String,
        onnx_execution_fingerprint: [u8; 32],
        auxiliary_execution_fingerprint: Option<[u8; 32]>,
        execution_threads: usize,
        logical_capacity: usize,
    ) -> Result<ResourceKey, serde_json::Error> {
        let identity = PhysicalResourceIdentity {
            adapter: self.adapter,
            artifact_fingerprint,
            onnx_execution_fingerprint,
            auxiliary_execution_fingerprint,
            execution_threads,
            physical_replicas: self.physical_capacity.resolve(logical_capacity),
            adapter_spec: &self.adapter_spec,
        };
        let bytes = serde_json::to_vec(&identity)?;
        use sha2::{Digest, Sha256};
        Ok(ResourceKey(Sha256::digest(bytes).into()))
    }
}

/// Canonical envelope serialized once for every local resource identity. Adapter-specific state
/// stays in `adapter_spec`; common execution state is never duplicated there. Only the resolved
/// physical replica count belongs here: application concurrency must not alter resource identity.
#[derive(Serialize)]
struct PhysicalResourceIdentity<'a> {
    adapter: &'static str,
    artifact_fingerprint: String,
    onnx_execution_fingerprint: [u8; 32],
    auxiliary_execution_fingerprint: Option<[u8; 32]>,
    execution_threads: usize,
    physical_replicas: usize,
    adapter_spec: &'a serde_json::Value,
}
