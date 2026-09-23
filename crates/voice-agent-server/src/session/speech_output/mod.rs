use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::config::SpeechOutputConfig;

use crate::{
    audio::{
        DOWNLINK_FRAME_SAMPLES, DownlinkOpusEncoder, DownlinkPcmFrame, DownlinkResampler,
        Pcm16Mono, PcmF32Mono,
    },
    providers::TtsProvider,
    workers::{TtsLease, TtsStreamId, TtsWorkerEvent, TtsWorkerRuntime},
};

const PROVIDER_SAMPLE_RATE_HZ: u32 = 48_000;
const FADE_IN_SAMPLES: usize = 48_000 * 8 / 1_000;
const PACED_FRAME_DURATION: Duration = Duration::from_millis(60);
const PREBUFFER_PACKETS: usize = 5;
const MAX_BUFFERED_PACKETS: usize = 32;

mod text;

use text::{JsonFilter, SentenceSegmenter, float_to_i16, sanitize_tts_text};

/// The actor observes these events; this module never owns a WebSocket sender.
pub enum SpeechOutputEvent {
    SegmentReady { text: String },
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
    tts_stream: Option<TtsStreamId>,
    active_worker: Option<TtsLease>,
    encoder: DownlinkOpusEncoder,
    pending: VecDeque<SpeechSegment>,
    downlink_resampler: DownlinkResampler,
    first_pcm_chunk: bool,
    downlink_tail: Vec<i16>,
    packets: VecDeque<Vec<u8>>,
    packets_sent: usize,
    finish_input: bool,
    started: bool,
    playback_origin: Option<Instant>,
    playback_end_deadline: Option<Instant>,
    json_filter: JsonFilter,
    segmenter: SentenceSegmenter,
    max_pending: usize,
}

struct SpeechSegment {
    display_text: String,
    tts_text: String,
    announced: bool,
}

mod pacing;
mod pipeline;
#[cfg(test)]
mod tests;

impl SpeechOutput {
    pub fn with_config(
        tts: Arc<dyn TtsProvider>,
        config: SpeechOutputConfig,
    ) -> Result<Self, crate::audio::AudioError> {
        let max_pending = config.pending_segments;
        Ok(Self {
            tts,
            tts_runtime: None,
            tts_stream: None,
            active_worker: None,
            encoder: DownlinkOpusEncoder::new(4_000)?,
            downlink_resampler: DownlinkResampler::new_48k_to_24k(),
            first_pcm_chunk: true,
            pending: VecDeque::new(),
            downlink_tail: Vec::new(),
            packets: VecDeque::new(),
            packets_sent: 0,
            finish_input: false,
            started: false,
            playback_origin: None,
            playback_end_deadline: None,
            json_filter: JsonFilter::default(),
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
        output.tts_stream = Some(runtime.begin_stream());
        output.tts_runtime = Some(runtime);
        Ok(output)
    }

    /// Accepts LLM text incrementally. Every completed segment is admitted atomically.
    pub fn push_delta(&mut self, text: &str) -> Result<(), SpeechOutputError> {
        let text = self.json_filter.push(text);
        for segment in self.segmenter.push(&text) {
            self.enqueue(segment)?;
        }
        // Emergency bound for an unfinished sentence; never synthesize a partial sentence.
        if self.segmenter.buffer.chars().count() + self.json_filter.candidate.chars().count()
            > self.segmenter.config.max_chars.saturating_mul(2)
        {
            return Err(SpeechOutputError::Backpressure);
        }
        Ok(())
    }

    pub fn finish_input(&mut self) -> Result<(), SpeechOutputError> {
        let remaining = self.json_filter.finish();
        for segment in self.segmenter.push(&remaining) {
            self.enqueue(segment)?;
        }
        if let Some(segment) = self.segmenter.finish() {
            self.enqueue(segment)?;
        }
        if !self.started
            && self.pending.is_empty()
            && self.active_worker.is_none()
            && self.packets.is_empty()
        {
            return Err(SpeechOutputError::Synthesis);
        }
        self.finish_input = true;
        Ok(())
    }
}
