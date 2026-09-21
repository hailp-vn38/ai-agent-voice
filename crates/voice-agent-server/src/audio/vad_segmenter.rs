/// Converts provider-level speech presence into stable utterance boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VadBoundary {
    SpeechStart,
    SpeechEnd,
}

#[derive(Default)]
pub struct VadSegmenter {
    detecting_speech: bool,
}

impl VadSegmenter {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn observe(&mut self, speech_detected: bool) -> Option<VadBoundary> {
        let boundary = match (self.detecting_speech, speech_detected) {
            (false, true) => Some(VadBoundary::SpeechStart),
            (true, false) => Some(VadBoundary::SpeechEnd),
            _ => None,
        };
        self.detecting_speech = speech_detected;
        boundary
    }
    pub fn reset(&mut self) {
        self.detecting_speech = false;
    }
}
