//! TTS provider boundary. Synthesis is added by the SpeechOutput ticket.

pub trait TtsProvider: Send + Sync {
    fn adapter(&self) -> &'static str;
}

pub struct UnavailableTts;

impl TtsProvider for UnavailableTts {
    fn adapter(&self) -> &'static str {
        "unavailable"
    }
}

pub(crate) struct ConfiguredZeroTts;

impl TtsProvider for ConfiguredZeroTts {
    fn adapter(&self) -> &'static str {
        "zerotts_onnx"
    }
}
