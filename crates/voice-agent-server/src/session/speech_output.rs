use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::config::SpeechOutputConfig;

use crate::{
    audio::{DOWNLINK_FRAME_SAMPLES, DownlinkOpusEncoder, DownlinkPcmFrame, Pcm16Mono, PcmF32Mono},
    providers::TtsProvider,
    workers::{TtsLease, TtsWorkerEvent, TtsWorkerRuntime},
};

const PROVIDER_SAMPLE_RATE_HZ: u32 = 48_000;
const PACED_FRAME_DURATION: Duration = Duration::from_millis(60);

/// The actor observes these events; this module never owns a WebSocket sender.
pub enum SpeechOutputEvent {
    Started,
    AudioPacket(Vec<u8>),
    Drained,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeechOutputError {
    Backpressure,
    Synthesis,
}

/// Owns synthesis, canonical conversion, Opus encoding, and audio pacing for one turn.
pub struct SpeechOutput {
    tts: Arc<dyn TtsProvider>,
    tts_runtime: Option<Arc<TtsWorkerRuntime>>,
    active_worker: Option<TtsLease>,
    encoder: DownlinkOpusEncoder,
    pending: VecDeque<String>,
    packets: VecDeque<Vec<u8>>,
    next_packet: usize,
    finish_input: bool,
    started: bool,
    next_deadline: Option<Instant>,
    segmenter: SentenceSegmenter,
    max_pending: usize,
}

impl SpeechOutput {
    pub fn with_config(
        tts: Arc<dyn TtsProvider>,
        config: SpeechOutputConfig,
    ) -> Result<Self, crate::audio::AudioError> {
        let max_pending = config.pending_segments;
        Ok(Self {
            tts,
            tts_runtime: None,
            active_worker: None,
            encoder: DownlinkOpusEncoder::new(4_000)?,
            pending: VecDeque::new(),
            packets: VecDeque::new(),
            next_packet: 0,
            finish_input: false,
            started: false,
            next_deadline: None,
            segmenter: SentenceSegmenter::new(config),
            max_pending,
        })
    }

    pub fn with_worker(
        tts: Arc<dyn TtsProvider>,
        runtime: Arc<TtsWorkerRuntime>,
        config: SpeechOutputConfig,
    ) -> Result<Self, crate::audio::AudioError> {
        let mut output = Self::with_config(tts, config)?;
        output.tts_runtime = Some(runtime);
        Ok(output)
    }

    /// Accepts LLM text incrementally. Every completed segment is admitted atomically.
    pub fn push_delta(&mut self, text: &str) -> Result<(), SpeechOutputError> {
        for segment in self.segmenter.push(text) {
            self.enqueue(segment)?;
        }
        Ok(())
    }

    pub fn finish_input(&mut self) -> Result<(), SpeechOutputError> {
        if let Some(segment) = self.segmenter.finish() {
            self.enqueue(segment)?;
        }
        self.finish_input = true;
        Ok(())
    }

    fn enqueue(&mut self, text: String) -> Result<(), SpeechOutputError> {
        // Encoded-but-not-paced audio remains delivery work, so it counts toward the same
        // semantic bound rather than turning a fast LLM into an unbounded packet buffer.
        if self.pending.len() + self.packets.len() >= self.max_pending {
            return Err(SpeechOutputError::Backpressure);
        }
        self.pending.push_back(text);
        Ok(())
    }

    fn synthesize_next(&mut self) -> Result<(), SpeechOutputError> {
        let Some(text) = self.pending.pop_front() else {
            return Ok(());
        };
        if let Some(runtime) = &self.tts_runtime {
            let lease = runtime
                .start(text)
                .map_err(|_| SpeechOutputError::Synthesis)?;
            self.active_worker = Some(lease);
            return Ok(());
        }
        let pcm = self
            .tts
            .synthesize(&text)
            .map_err(|_| SpeechOutputError::Synthesis)?;
        self.enqueue_pcm(pcm)
    }

    fn enqueue_pcm(&mut self, pcm: PcmF32Mono) -> Result<(), SpeechOutputError> {
        let downlink = resample_to_downlink(pcm).ok_or(SpeechOutputError::Synthesis)?;
        for samples in downlink.chunks(DOWNLINK_FRAME_SAMPLES) {
            let mut frame = samples.to_vec();
            frame.resize(DOWNLINK_FRAME_SAMPLES, 0);
            let packet = self
                .encoder
                .encode(
                    DownlinkPcmFrame::try_new(Pcm16Mono::new(frame))
                        .map_err(|_| SpeechOutputError::Synthesis)?,
                )
                .map_err(|_| SpeechOutputError::Synthesis)?;
            self.packets.push_back(packet.as_bytes().to_vec());
        }
        if self.packets.is_empty() {
            return Err(SpeechOutputError::Synthesis);
        }
        Ok(())
    }

    pub fn cancel(&mut self) {
        if let Some(lease) = self.active_worker.take()
            && let Some(runtime) = &self.tts_runtime
        {
            let _ = runtime.cancel_and_detach(lease);
        }
        self.pending.clear();
        self.packets.clear();
        self.next_packet = 0;
        self.finish_input = false;
        self.started = false;
        self.next_deadline = None;
        self.segmenter.reset();
    }

    pub fn poll(&mut self) -> Result<Option<SpeechOutputEvent>, SpeechOutputError> {
        // Only one segment is synthesized at a time; packet pacing may overlap the next poll.
        if self.packets.is_empty() && self.active_worker.is_none() && !self.pending.is_empty() {
            self.synthesize_next()?;
            // Give a just-dispatched native worker one scheduling opportunity without running
            // inference on the actor/Tokio thread.
            std::thread::yield_now();
        }
        if let Some(lease) = self.active_worker {
            let event = self
                .tts_runtime
                .as_ref()
                .expect("active TTS worker requires a runtime")
                .poll(lease)
                .map_err(|_| SpeechOutputError::Synthesis)?;
            match event {
                None => {}
                Some(TtsWorkerEvent::Pcm(pcm)) => self.enqueue_pcm(pcm)?,
                Some(TtsWorkerEvent::Finished) => self.active_worker = None,
                Some(
                    TtsWorkerEvent::Cancelled
                    | TtsWorkerEvent::Failed
                    | TtsWorkerEvent::CleanupTimedOut,
                ) => {
                    self.active_worker = None;
                    return Err(SpeechOutputError::Synthesis);
                }
                Some(TtsWorkerEvent::TimedOut) => {
                    self.active_worker = None;
                    // Actor failure stops polling this lease, so cleanup must continue at the
                    // runtime boundary and quarantine the slot if native acknowledgement stalls.
                    let _ = self
                        .tts_runtime
                        .as_ref()
                        .expect("active TTS worker requires a runtime")
                        .cancel_and_detach(lease);
                    return Err(SpeechOutputError::Synthesis);
                }
            }
        }
        if self.next_packet == self.packets.len() {
            if self.finish_input
                && self.pending.is_empty()
                && self.active_worker.is_none()
                && self.started
                && self
                    .next_deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
            {
                self.cancel();
                return Ok(Some(SpeechOutputEvent::Drained));
            }
            return Ok(None);
        }
        if self
            .next_deadline
            .is_some_and(|deadline| Instant::now() < deadline)
        {
            return Ok(None);
        }
        if !self.started {
            self.started = true;
            return Ok(Some(SpeechOutputEvent::Started));
        }
        let packet = self.packets.pop_front().expect("checked non-empty");
        self.next_packet = 0;
        self.next_deadline = Some(Instant::now() + PACED_FRAME_DURATION);
        Ok(Some(SpeechOutputEvent::AudioPacket(packet)))
    }
}

/// Pure V1 sentence delivery policy. It never knows about synthesis or transport.
struct SentenceSegmenter {
    config: SpeechOutputConfig,
    buffer: String,
}

impl SentenceSegmenter {
    fn new(config: SpeechOutputConfig) -> Self {
        Self {
            config,
            buffer: String::new(),
        }
    }
    fn reset(&mut self) {
        self.buffer.clear();
    }
    fn push(&mut self, delta: &str) -> Vec<String> {
        self.buffer.push_str(delta);
        let mut segments = Vec::new();
        loop {
            let chars = self.buffer.chars().count();
            let boundary =
                self.buffer
                    .char_indices()
                    .enumerate()
                    .find_map(|(char_index, (index, ch))| {
                        let before = char_index + 1;
                        let hard =
                            matches!(ch, '.' | '!' | '?' | '\u{3002}' | '\u{ff01}' | '\u{ff1f}');
                        let soft = matches!(
                            ch,
                            ',' | ';' | ':' | '\u{ff0c}' | '\u{3001}' | '\u{ff1b}' | '\u{ff1a}'
                        );
                        ((hard && before >= self.config.min_chars)
                            || (soft && before >= self.config.soft_break_min_chars))
                            .then_some(index + ch.len_utf8())
                    });
            let split = boundary
                .or_else(|| (chars >= self.config.max_chars).then(|| self.preferred_split()));
            let Some(split) = split else { break };
            let segment = self.buffer[..split].trim().to_owned();
            self.buffer.drain(..split);
            if !segment.is_empty() {
                segments.push(segment);
            }
        }
        segments
    }
    fn preferred_split(&self) -> usize {
        let limit = self
            .buffer
            .char_indices()
            .nth(self.config.max_chars)
            .map_or(self.buffer.len(), |(i, _)| i);
        self.buffer[..limit]
            .rfind(char::is_whitespace)
            .filter(|i| *i > 0)
            .map_or(limit, |i| i + 1)
    }
    fn finish(&mut self) -> Option<String> {
        let text = self.buffer.trim().to_owned();
        self.buffer.clear();
        (!text.is_empty()).then_some(text)
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::SentenceSegmenter;
    use crate::config::SpeechOutputConfig;

    #[test]
    fn splits_unicode_at_hard_punctuation_and_whitespace_before_hard_limit() {
        let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig {
            min_chars: 4,
            soft_break_min_chars: 8,
            max_chars: 10,
            pending_segments: 2,
        });
        assert_eq!(segmenter.push("Xin chao。Tiep"), ["Xin chao。"]);
        assert_eq!(segmenter.push(" tuc rat dai"), ["Tiep tuc"]);
        assert_eq!(segmenter.finish(), Some("rat dai".into()));
    }
}

fn resample_to_downlink(pcm: PcmF32Mono) -> Option<Vec<i16>> {
    if pcm.sample_rate_hz() != PROVIDER_SAMPLE_RATE_HZ || pcm.samples().is_empty() {
        return None;
    }
    let samples = pcm.samples();
    let mut output = Vec::with_capacity(samples.len() / 2);
    let (pairs, _) = samples.as_chunks::<2>();
    for pair in pairs {
        let sample = (pair[0] + pair[1]) * 0.5;
        if !sample.is_finite() {
            return None;
        }
        output.push((sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16);
    }
    (!output.is_empty()).then_some(output)
}
