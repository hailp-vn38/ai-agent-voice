//! CAM++ extraction only; matching and authorization belong to domain services.
pub mod assets;
pub mod descriptor;

use crate::audio::PcmF32Mono;
pub use crate::workers::SpeakerRuntime;

#[derive(Debug, thiserror::Error)]
pub enum SpeakerError {
    #[error("speaker input is invalid")]
    InvalidInput,
    #[error("speaker runtime is busy")]
    Busy,
    #[error("speaker runtime is unavailable")]
    Unavailable,
    #[error("speaker returned an invalid embedding")]
    InvalidEmbedding,
}

/// Qualification providers implement the same extractor contract without native models.
pub trait SpeakerProvider: Send + 'static {
    fn dimension(&self) -> usize;
    fn extract(&mut self, pcm: &PcmF32Mono) -> Result<Vec<f32>, SpeakerError>;
}

struct CampPlus(sherpa_onnx::SpeakerEmbeddingExtractor);
impl SpeakerProvider for CampPlus {
    fn dimension(&self) -> usize {
        self.0.dim() as usize
    }
    fn extract(&mut self, pcm: &PcmF32Mono) -> Result<Vec<f32>, SpeakerError> {
        let stream = self.0.create_stream().ok_or(SpeakerError::Unavailable)?;
        stream.accept_waveform(16_000, pcm.samples());
        stream.input_finished();
        if !self.0.is_ready(&stream) {
            return Err(SpeakerError::InvalidInput);
        }
        self.0.compute(&stream).ok_or(SpeakerError::Unavailable)
    }
}

pub fn build(threads: i32) -> Result<Box<dyn SpeakerProvider>, SpeakerError> {
    if !(1..=128).contains(&threads) {
        return Err(SpeakerError::InvalidInput);
    }
    let model = assets::resolve_assets().map_err(|_| SpeakerError::Unavailable)?;
    let config = sherpa_onnx::SpeakerEmbeddingExtractorConfig {
        model: Some(model.display().to_string()),
        num_threads: threads,
        ..Default::default()
    };
    let extractor =
        sherpa_onnx::SpeakerEmbeddingExtractor::create(&config).ok_or(SpeakerError::Unavailable)?;
    if !(1..=4096).contains(&extractor.dim()) {
        return Err(SpeakerError::InvalidEmbedding);
    }
    Ok(Box::new(CampPlus(extractor)))
}
