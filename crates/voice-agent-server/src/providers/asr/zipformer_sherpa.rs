use std::sync::Arc;

use sherpa_onnx::{OnlineRecognizer, OnlineStream};

use crate::{
    audio::PcmF32Mono,
    providers::{AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession},
};

// sherpa-onnx's streaming file examples append 0.66 s before InputFinished so the
// encoder can consume its final right-context window. Without it, a Manual stop at
// the exact end of speech can leave the last tokens undecoded.
const FINAL_PADDING_SAMPLES: usize = 10_560;
static FINAL_PADDING: [f32; FINAL_PADDING_SAMPLES] = [0.0; FINAL_PADDING_SAMPLES];

pub(crate) struct UnavailableAsr;

impl AsrProvider for UnavailableAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(UnavailableAsrSession))
    }
}

struct UnavailableAsrSession;

impl AsrSession for UnavailableAsrSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(Vec::new())
    }
    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Err(AsrError::Failed("ASR provider is not initialized".into()))
    }
    fn cancel(&mut self) {}
}

pub(crate) struct ZipformerAsrProvider {
    pub(crate) recognizer: Arc<OnlineRecognizer>,
}

impl AsrProvider for ZipformerAsrProvider {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(ZipformerAsrSession {
            recognizer: Arc::clone(&self.recognizer),
            stream: self.recognizer.create_stream(),
        }))
    }
}

struct ZipformerAsrSession {
    recognizer: Arc<OnlineRecognizer>,
    stream: OnlineStream,
}

impl AsrSession for ZipformerAsrSession {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        if pcm.sample_rate_hz() != 16_000 {
            return Err(AsrError::Failed("ASR requires canonical 16 kHz PCM".into()));
        }
        self.stream
            .accept_waveform(pcm.sample_rate_hz() as i32, pcm.samples());
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }
        Ok(self
            .recognizer
            .get_result(&self.stream)
            .filter(|result| !result.text.trim().is_empty())
            .map(|result| vec![AsrEvent::Partial(result.text)])
            .unwrap_or_default())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        self.stream.accept_waveform(16_000, &FINAL_PADDING);
        self.stream.input_finished();
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }
        self.recognizer
            .get_result(&self.stream)
            .map(|result| AsrResult::new(result.text))
            .ok_or_else(|| AsrError::Failed("Zipformer returned no final result".into()))
    }

    fn cancel(&mut self) {}
}
