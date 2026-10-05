use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::config::SpeechOutputConfig;

use crate::{
    audio::{CanonicalDownlinkPipeline, MAX_DOWNLINK_OPUS_PACKET_BYTES, PcmF32Mono},
    providers::TtsProvider,
    workers::{TtsLease, TtsStreamId, TtsWorkerEvent, TtsWorkerRuntime},
};

const PACED_FRAME_DURATION: Duration = Duration::from_millis(60);
const PREBUFFER_PACKETS: usize = 5;
const MAX_BUFFERED_PACKETS: usize = 32;

mod text;

use text::{JsonFilter, SentenceSegmenter, sanitize_tts_text};

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
    downlink_pipeline: CanonicalDownlinkPipeline,
    pending: VecDeque<SpeechSegment>,
    packets: VecDeque<Vec<u8>>,
    packets_sent: usize,
    finish_input: bool,
    started: bool,
    playback_origin: Option<Instant>,
    audio_starved_at: Option<Instant>,
    playback_end_deadline: Option<Instant>,
    json_filter: JsonFilter,
    segmenter: SentenceSegmenter,
    pending_filtered_text: String,
    finish_requested: bool,
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
    pub(crate) fn has_pending_capacity(&self) -> bool {
        self.pending.len() < self.max_pending && self.pending_filtered_text.is_empty()
    }

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
            downlink_pipeline: CanonicalDownlinkPipeline::new(MAX_DOWNLINK_OPUS_PACKET_BYTES)?,
            pending: VecDeque::new(),
            packets: VecDeque::new(),
            packets_sent: 0,
            finish_input: false,
            started: false,
            playback_origin: None,
            audio_starved_at: None,
            playback_end_deadline: None,
            json_filter: JsonFilter::default(),
            segmenter: SentenceSegmenter::new(config),
            pending_filtered_text: String::new(),
            finish_requested: false,
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

    /// Releases the worker stream this output holds, for an owner that replaces it wholesale.
    ///
    /// `cancel` deliberately reopens a stream for the next turn; a replaced output has no next
    /// turn on that runtime, so its stream must be closed or the runtime registry grows per switch.
    pub fn release(&mut self) {
        if let (Some(runtime), Some(stream)) = (&self.tts_runtime, self.tts_stream.take()) {
            runtime.close_stream(stream);
        }
    }

    /// Accepts LLM text incrementally. Every completed segment is admitted atomically.
    pub fn push_delta(&mut self, text: &str) -> Result<(), SpeechOutputError> {
        let text = self.json_filter.push(text);
        self.pending_filtered_text.push_str(&text);
        self.advance_filtered_text()
    }

    pub fn finish_input(&mut self) -> Result<(), SpeechOutputError> {
        self.pending_filtered_text
            .push_str(&self.json_filter.finish());
        self.finish_requested = true;
        self.advance_filtered_text()?;
        Ok(())
    }

    fn advance_filtered_text(&mut self) -> Result<(), SpeechOutputError> {
        let text = std::mem::take(&mut self.pending_filtered_text);
        for (offset, ch) in text.char_indices() {
            if self.pending.len() >= self.max_pending {
                self.pending_filtered_text.push_str(&text[offset..]);
                return Ok(());
            }
            for segment in self.segmenter.push(ch.encode_utf8(&mut [0; 4])) {
                self.enqueue(segment)?;
            }
        }
        if self.finish_requested && self.pending.len() < self.max_pending {
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
        }
        Ok(())
    }
}
