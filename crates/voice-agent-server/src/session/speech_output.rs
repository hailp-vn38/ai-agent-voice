use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::config::SpeechOutputConfig;
use unicode_normalization::UnicodeNormalization;

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
    next_packet: usize,
    finish_input: bool,
    started: bool,
    next_deadline: Option<Instant>,
    segmenter: SentenceSegmenter,
    max_pending: usize,
}

struct SpeechSegment {
    display_text: String,
    tts_text: String,
    announced: bool,
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
            tts_stream: None,
            active_worker: None,
            encoder: DownlinkOpusEncoder::new(4_000)?,
            downlink_resampler: DownlinkResampler::new_48k_to_24k(),
            first_pcm_chunk: true,
            pending: VecDeque::new(),
            downlink_tail: Vec::new(),
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
        output.tts_stream = Some(runtime.begin_stream());
        output.tts_runtime = Some(runtime);
        Ok(output)
    }

    /// Accepts LLM text incrementally. Every completed segment is admitted atomically.
    pub fn push_delta(&mut self, text: &str) -> Result<(), SpeechOutputError> {
        for segment in self.segmenter.push(text) {
            self.enqueue(segment)?;
        }
        // Emergency bound for an unfinished sentence; never synthesize a partial sentence.
        if self.segmenter.buffer.chars().count() > self.segmenter.config.max_chars.saturating_mul(2)
        {
            return Err(SpeechOutputError::Backpressure);
        }
        Ok(())
    }

    pub fn finish_input(&mut self) -> Result<(), SpeechOutputError> {
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

    fn enqueue(&mut self, text: String) -> Result<(), SpeechOutputError> {
        let display_text = text.trim().to_owned();
        let tts_text = sanitize_tts_text(&display_text);
        if display_text.is_empty() || tts_text.is_empty() {
            return Ok(());
        }
        // This is the semantic bound for text awaiting synthesis. Audio has its own bounded
        // transport queue, so a long valid segment must not consume future segment capacity.
        if self.pending.len() >= self.max_pending {
            return Err(SpeechOutputError::Backpressure);
        }
        self.pending.push_back(SpeechSegment {
            display_text,
            tts_text,
            announced: false,
        });
        Ok(())
    }

    fn synthesize_next(&mut self) -> Result<(), SpeechOutputError> {
        let Some(segment) = self.pending.pop_front() else {
            return Ok(());
        };
        let text = segment.tts_text;
        if let Some(runtime) = &self.tts_runtime {
            let stream = self.tts_stream.expect("TTS worker requires a stream");
            tracing::info!(?stream, "TTS segment submission requested");
            let lease = runtime
                .start_in_stream(stream, text)
                .map_err(|_| SpeechOutputError::Synthesis)?;
            self.active_worker = Some(lease);
            return Ok(());
        }
        tracing::info!(
            tts_input = %text,
            chars = text.chars().count(),
            delivery = "direct",
            "TTS synthesis input"
        );
        let pcm = self
            .tts
            .synthesize(&text)
            .map_err(|_| SpeechOutputError::Synthesis)?;
        self.push_provider_pcm(pcm)
    }

    fn push_provider_pcm(&mut self, mut pcm: PcmF32Mono) -> Result<(), SpeechOutputError> {
        if pcm.sample_rate_hz() != PROVIDER_SAMPLE_RATE_HZ || pcm.samples().is_empty() {
            return Err(SpeechOutputError::Synthesis);
        }
        if self.first_pcm_chunk {
            let fade_samples = FADE_IN_SAMPLES.min(pcm.samples().len());
            for (index, sample) in pcm.samples_mut()[..fade_samples].iter_mut().enumerate() {
                *sample *= index as f32 / fade_samples as f32;
            }
            self.first_pcm_chunk = false;
        }
        let downlink = self
            .downlink_resampler
            .process(pcm.samples())
            .map_err(|_| SpeechOutputError::Synthesis)?
            .into_iter()
            .map(float_to_i16)
            .collect::<Vec<_>>();
        self.downlink_tail.extend(downlink);
        while self.downlink_tail.len() >= DOWNLINK_FRAME_SAMPLES {
            let frame = self
                .downlink_tail
                .drain(..DOWNLINK_FRAME_SAMPLES)
                .collect::<Vec<_>>();
            let packet = self
                .encoder
                .encode(
                    DownlinkPcmFrame::try_new(Pcm16Mono::new(frame))
                        .map_err(|_| SpeechOutputError::Synthesis)?,
                )
                .map_err(|_| SpeechOutputError::Synthesis)?;
            if !self.started && self.packets.is_empty() {
                tracing::info!(
                    packet_bytes = packet.as_bytes().len(),
                    "First downlink Opus packet ready"
                );
            }
            self.packets.push_back(packet.as_bytes().to_vec());
        }
        Ok(())
    }

    fn flush_downlink_tail(&mut self) -> Result<(), SpeechOutputError> {
        if self.downlink_tail.is_empty() {
            return Ok(());
        }
        for sample in self.downlink_resampler.flush() {
            self.downlink_tail.push(float_to_i16(sample));
        }
        self.downlink_tail.resize(DOWNLINK_FRAME_SAMPLES, 0);
        let frame = std::mem::take(&mut self.downlink_tail);
        let packet = self
            .encoder
            .encode(
                DownlinkPcmFrame::try_new(Pcm16Mono::new(frame))
                    .map_err(|_| SpeechOutputError::Synthesis)?,
            )
            .map_err(|_| SpeechOutputError::Synthesis)?;
        self.packets.push_back(packet.as_bytes().to_vec());
        Ok(())
    }

    pub fn cancel(&mut self) {
        if let Some(lease) = self.active_worker.take()
            && let Some(runtime) = &self.tts_runtime
        {
            let _ = runtime.cancel_and_detach(lease);
        }
        self.pending.clear();
        self.downlink_tail.clear();
        self.downlink_resampler = DownlinkResampler::new_48k_to_24k();
        self.first_pcm_chunk = true;
        self.packets.clear();
        self.next_packet = 0;
        self.finish_input = false;
        self.started = false;
        self.next_deadline = None;
        self.segmenter.reset();
        if let (Some(runtime), Some(stream)) = (&self.tts_runtime, self.tts_stream.take()) {
            runtime.close_stream(stream);
            self.tts_stream = Some(runtime.begin_stream());
        }
    }

    pub fn poll(&mut self) -> Result<Option<SpeechOutputEvent>, SpeechOutputError> {
        // Only one segment is synthesized at a time; packet pacing may overlap the next poll.
        if self.packets.is_empty() && self.active_worker.is_none() && !self.pending.is_empty() {
            let segment = self.pending.front_mut().expect("checked non-empty");
            if !segment.announced {
                segment.announced = true;
                tracing::info!(text = %segment.display_text, "Speech segment ready");
                return Ok(Some(SpeechOutputEvent::SegmentReady {
                    text: segment.display_text.clone(),
                }));
            }
            self.synthesize_next()?;
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
                Some(TtsWorkerEvent::Pcm(pcm)) => self.push_provider_pcm(pcm)?,
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
        if self.next_packet == self.packets.len()
            && self.finish_input
            && self.pending.is_empty()
            && self.active_worker.is_none()
        {
            self.flush_downlink_tail()?;
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
            let boundary = self.buffer.char_indices().find_map(|(index, ch)| {
                let next = self.buffer[index + ch.len_utf8()..].chars().next();
                let previous = self.buffer[..index].chars().next_back();
                // A trailing digit-dot may become a decimal or version separator in
                // the next delta. A trailing word-dot is released immediately so a
                // complete sentence can start audio before LLM EOF.
                let sentence_dot = ch == '.'
                    && match next {
                        Some(next) => {
                            next.is_whitespace()
                                || matches!(next, '"' | '\'' | ')' | ']' | '”' | '’')
                        }
                        None => previous.is_some_and(|previous| !previous.is_ascii_digit()),
                    };
                let hard =
                    sentence_dot || matches!(ch, '!' | '?' | '\u{3002}' | '\u{ff01}' | '\u{ff1f}');
                hard.then_some(index + ch.len_utf8())
            });
            let Some(split) = boundary else { break };
            let segment = self.buffer[..split].trim().to_owned();
            self.buffer.drain(..split);
            if !segment.is_empty() {
                segments.push(segment);
            }
        }
        segments
    }
    fn finish(&mut self) -> Option<String> {
        let text = self.buffer.trim().to_owned();
        self.buffer.clear();
        (!text.is_empty()).then_some(text)
    }
}

fn sanitize_tts_text(input: &str) -> String {
    let normalized = input.nfc().collect::<String>();
    let mut output = String::with_capacity(normalized.len());
    let mut pending_space = false;
    for ch in normalized.chars() {
        if ch.is_alphanumeric() {
            if pending_space && !output.is_empty() {
                output.push(' ');
            }
            pending_space = false;
            output.push(ch);
        } else if ch.is_whitespace() || matches!(ch, '_' | '-' | '–' | '—') {
            pending_space = true;
        } else if matches!(ch, '.' | ',' | '!' | '?' | ';' | ':') {
            pending_space = false;
            output.push(ch);
        }
    }
    output.trim().to_owned()
}

fn float_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

#[cfg(test)]
mod tests {
    use super::{SentenceSegmenter, sanitize_tts_text};
    use crate::config::SpeechOutputConfig;

    #[test]
    fn splits_unicode_only_at_complete_sentence_boundary() {
        let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig {
            min_chars: 4,
            soft_break_min_chars: 8,
            max_chars: 10,
            pending_segments: 2,
        });
        assert_eq!(segmenter.push("Xin chao。Tiep"), ["Xin chao。"]);
        assert!(segmenter.push(" tuc rat dai").is_empty());
        assert_eq!(segmenter.finish(), Some("Tiep tuc rat dai".into()));
    }

    #[test]
    fn holds_fragmented_vietnamese_deltas_until_the_complete_sentence_boundary() {
        let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig::default());

        for delta in ["Xin", " ch", "ào"] {
            assert!(
                segmenter.push(delta).is_empty(),
                "fragment {delta:?} must not be submitted to TTS"
            );
        }
        assert_eq!(segmenter.push("!"), ["Xin chào!"]);
        for delta in [
            " M", "ình", " có", " thể", " giúp", " gì", " cho", " bạn", " hôm", " nay",
        ] {
            assert!(segmenter.push(delta).is_empty());
        }

        assert_eq!(
            segmenter.push("?"),
            ["Mình có thể giúp gì cho bạn hôm nay?"]
        );
        assert_eq!(segmenter.finish(), None);
    }

    #[test]
    fn does_not_send_a_partial_version_to_tts_when_decimal_dots_span_deltas() {
        let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig::default());
        assert!(segmenter.push("Phiên bản phần mềm hiện là v1.").is_empty());
        assert_eq!(
            segmenter.push("2.3 và đã sẵn sàng."),
            ["Phiên bản phần mềm hiện là v1.2.3 và đã sẵn sàng."]
        );
        assert_eq!(segmenter.finish(), None);
    }

    #[test]
    fn emits_a_complete_sentence_before_the_llm_finishes() {
        let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig::default());
        assert_eq!(
            segmenter.push("Phản hồi âm thanh hoàn chỉnh cho Reference Client."),
            ["Phản hồi âm thanh hoàn chỉnh cho Reference Client."]
        );
        assert_eq!(segmenter.finish(), None);
    }

    #[test]
    fn holds_a_long_sentence_until_eof() {
        let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig {
            min_chars: 4,
            soft_break_min_chars: 8,
            max_chars: 10,
            pending_segments: 2,
        });
        assert!(segmenter.push("abcdefghij").is_empty());
        assert!(segmenter.push("klm next").is_empty());
        assert_eq!(segmenter.finish(), Some("abcdefghijklm next".into()));
    }

    #[test]
    fn rejects_a_token_too_long_to_split_safely() {
        use super::{SpeechOutput, SpeechOutputError};
        use crate::providers::tts::UnavailableTts;
        use std::sync::Arc;

        let mut output = SpeechOutput::with_config(
            Arc::new(UnavailableTts),
            SpeechOutputConfig {
                min_chars: 4,
                soft_break_min_chars: 8,
                max_chars: 10,
                pending_segments: 2,
            },
        )
        .unwrap();
        assert_eq!(
            output.push_delta("abcdefghijklmnopqrstu"),
            Err(SpeechOutputError::Backpressure)
        );
    }

    #[test]
    fn short_hard_sentence_is_flushed_immediately() {
        let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig::default());
        for delta in ["Xin", " ch", "ào"] {
            assert!(segmenter.push(delta).is_empty());
        }
        assert_eq!(segmenter.push("!"), ["Xin chào!"]);
        assert!(
            segmenter
                .push(" Mình có thể giúp gì cho bạn hôm nay")
                .is_empty()
        );
        assert_eq!(
            segmenter.push("?"),
            ["Mình có thể giúp gì cho bạn hôm nay?"]
        );
    }

    #[test]
    fn soft_punctuation_and_length_do_not_split_unfinished_sentence() {
        let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig {
            min_chars: 4,
            soft_break_min_chars: 8,
            max_chars: 10,
            pending_segments: 2,
        });
        assert!(
            segmenter
                .push("Nếu bạn muốn, mình có thể kiểm tra")
                .is_empty()
        );
        assert_eq!(
            segmenter.push(" ngay bây giờ."),
            ["Nếu bạn muốn, mình có thể kiểm tra ngay bây giờ."]
        );
    }

    #[test]
    fn sanitizes_markdown_and_emoji_for_tts() {
        assert_eq!(sanitize_tts_text("**Xin chào!** 😊"), "Xin chào!");
        assert_eq!(
            sanitize_tts_text("### Kết quả:\n- Bạn có thể **khởi động lại** server 🚀"),
            "Kết quả: Bạn có thể khởi động lại server"
        );
        assert_eq!(
            sanitize_tts_text("Mình có thể giúp bạn #hôm_nay?"),
            "Mình có thể giúp bạn hôm nay?"
        );
    }

    #[test]
    fn segment_ready_keeps_display_text_while_tts_receives_sanitized_text() {
        use super::{SpeechOutput, SpeechOutputEvent};
        use crate::{
            audio::PcmF32Mono,
            providers::{TtsError, TtsProvider},
        };
        use std::sync::{Arc, Mutex};

        struct RecordingTts(Arc<Mutex<Vec<String>>>);
        impl TtsProvider for RecordingTts {
            fn adapter(&self) -> &'static str {
                "recording_tts"
            }
            fn synthesize(&self, text: &str) -> Result<PcmF32Mono, TtsError> {
                self.0.lock().unwrap().push(text.to_owned());
                Ok(PcmF32Mono::new(vec![0.1; 2_880], 48_000))
            }
        }

        let inputs = Arc::new(Mutex::new(Vec::new()));
        let mut output = SpeechOutput::with_config(
            Arc::new(RecordingTts(Arc::clone(&inputs))),
            SpeechOutputConfig::default(),
        )
        .unwrap();
        output.push_delta("**Xin chào!** 😊").unwrap();
        output.finish_input().unwrap();
        assert!(
            matches!(output.poll().unwrap(), Some(SpeechOutputEvent::SegmentReady { text }) if text == "**Xin chào!")
        );
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::Started)
        ));
        assert_eq!(*inputs.lock().unwrap(), ["Xin chào!"]);
    }

    #[test]
    fn unspeakable_response_fails_instead_of_waiting_forever() {
        use super::{SpeechOutput, SpeechOutputError};
        use crate::providers::tts::UnavailableTts;
        use std::sync::Arc;

        let mut output =
            SpeechOutput::with_config(Arc::new(UnavailableTts), SpeechOutputConfig::default())
                .unwrap();
        output.push_delta("😊🚀").unwrap();
        assert_eq!(output.finish_input(), Err(SpeechOutputError::Synthesis));
    }
}
