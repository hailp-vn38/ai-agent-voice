use sherpa_onnx::{VadModelConfig, VoiceActivityDetector};

use crate::{
    audio::{PcmF32Mono, VadBoundary, VadSegmenter},
    providers::{VadError, VadEvent, VadProvider, VadSession},
};

pub(crate) struct UnavailableVad;

impl VadProvider for UnavailableVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Err(VadError::Failed("VAD provider is not initialized".into()))
    }
    fn adapter(&self) -> &'static str {
        "unavailable"
    }
}

pub(crate) struct LoadedSileroVad {
    pub(crate) config: VadModelConfig,
}

impl VadProvider for LoadedSileroVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        let detector = VoiceActivityDetector::create(&self.config, 60.0)
            .ok_or_else(|| VadError::Failed("cannot initialize Silero VAD session".into()))?;
        Ok(Box::new(SileroVadSession {
            detector,
            segmenter: VadSegmenter::new(),
        }))
    }
    fn adapter(&self) -> &'static str {
        "silero_onnx"
    }
}

struct SileroVadSession {
    detector: VoiceActivityDetector,
    segmenter: VadSegmenter,
}

impl VadSession for SileroVadSession {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<VadEvent>, VadError> {
        if pcm.sample_rate_hz() != 16_000 {
            return Err(VadError::Failed("VAD requires canonical 16 kHz PCM".into()));
        }
        self.detector.accept_waveform(pcm.samples());
        Ok(match self.segmenter.observe(self.detector.detected()) {
            Some(VadBoundary::SpeechStart) => vec![VadEvent::SpeechStart],
            Some(VadBoundary::SpeechEnd) => vec![VadEvent::SpeechEnd],
            None => Vec::new(),
        })
    }

    fn reset(&mut self) -> Result<(), VadError> {
        self.detector.reset();
        self.segmenter.reset();
        Ok(())
    }
    fn close(&mut self) -> Result<(), VadError> {
        Ok(())
    }
}
