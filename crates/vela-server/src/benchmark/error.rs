use serde::{Serialize, Serializer};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BenchmarkErrorCategory {
    Config,
    ModelPreparation,
    ProviderBuild,
    Warmup,
    Synthesis,
    InvalidPcm,
    Resample,
    OpusInit,
    OpusEncode,
    WorkerReset,
    OutputIo,
}
impl Serialize for BenchmarkErrorCategory {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
impl BenchmarkErrorCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::ModelPreparation => "model_preparation",
            Self::ProviderBuild => "provider_build",
            Self::Warmup => "warmup",
            Self::Synthesis => "synthesis",
            Self::InvalidPcm => "invalid_pcm",
            Self::Resample => "resample",
            Self::OpusInit => "opus_init",
            Self::OpusEncode => "opus_encode",
            Self::WorkerReset => "worker_reset",
            Self::OutputIo => "output_io",
        }
    }
}
