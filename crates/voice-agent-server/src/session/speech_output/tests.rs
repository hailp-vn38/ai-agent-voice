use super::{SentenceSegmenter, sanitize_tts_text};
use crate::config::SpeechOutputConfig;

#[test]
fn short_streaming_segment_waits_for_completion_before_starting_playback() {
    use super::{SpeechOutput, SpeechOutputEvent};
    use crate::{
        audio::PcmF32Mono,
        providers::{TtsError, TtsProvider, TtsWorker},
        workers::{TtsWorkerRuntime, WorkerRuntimeConfig},
    };
    use std::{
        sync::{Arc, Mutex, atomic::AtomicBool, mpsc},
        time::Duration,
    };

    struct GatedProvider {
        emitted: mpsc::Sender<usize>,
        release: Arc<Mutex<mpsc::Receiver<()>>>,
    }
    struct GatedWorker {
        emitted: mpsc::Sender<usize>,
        release: Arc<Mutex<mpsc::Receiver<()>>>,
    }
    impl TtsProvider for GatedProvider {
        fn adapter(&self) -> &'static str {
            "gated"
        }
        fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
            Ok(Box::new(GatedWorker {
                emitted: self.emitted.clone(),
                release: Arc::clone(&self.release),
            }))
        }
    }
    impl TtsWorker for GatedWorker {
        fn synthesize(
            &mut self,
            _: &str,
            _: &AtomicBool,
            on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
        ) -> Result<(), TtsError> {
            for frame in 1..=5 {
                on_pcm(PcmF32Mono::new(vec![0.1; 2_880], 48_000))?;
                self.emitted.send(frame).unwrap();
                self.release.lock().unwrap().recv().unwrap();
            }
            Ok(())
        }
        fn reset(&mut self) -> Result<(), TtsError> {
            Ok(())
        }
    }

    let (emitted_tx, emitted_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let provider: Arc<dyn TtsProvider> = Arc::new(GatedProvider {
        emitted: emitted_tx,
        release: Arc::new(Mutex::new(release_rx)),
    });
    let runtime = Arc::new(TtsWorkerRuntime::new(
        Arc::clone(&provider),
        WorkerRuntimeConfig {
            max_workers: 1,
            command_capacity: 4,
            final_timeout: Duration::from_secs(2),
            cleanup_grace: Duration::from_secs(1),
        },
    ));
    let mut output =
        SpeechOutput::with_worker(provider, runtime, SpeechOutputConfig::default()).unwrap();
    output
        .push_delta("Một câu có âm thanh phát từng phần.")
        .unwrap();
    assert!(matches!(
        output.poll().unwrap(),
        Some(SpeechOutputEvent::SegmentReady { .. })
    ));
    assert!(output.poll().unwrap().is_none());
    for expected in 1..=5 {
        if expected > 1 {
            release_tx.send(()).unwrap();
        }
        assert_eq!(
            emitted_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            expected
        );
        for _ in 0..10_000 {
            let event = output.poll().unwrap();
            if output.packets.len() == expected {
                assert!(event.is_none(), "Started while worker still has more PCM");
                break;
            }
            assert!(
                event.is_none(),
                "Started before packet {expected} reached the queue"
            );
            std::thread::yield_now();
        }
        assert_eq!(output.packets.len(), expected);
        assert!(
            output.poll().unwrap().is_none(),
            "Started with worker active at packet {expected}"
        );
    }
    release_tx.send(()).unwrap();
    let mut started = false;
    for _ in 0..10_000 {
        if matches!(output.poll().unwrap(), Some(SpeechOutputEvent::Started)) {
            started = true;
            break;
        }
        std::thread::yield_now();
    }
    assert!(
        started,
        "short segment did not start after worker completion"
    );
    for _ in 0..5 {
        assert!(matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::AudioPacket(_))
        ));
    }
}

#[test]
fn long_streaming_segment_starts_at_bounded_initial_buffer() {
    use super::{MAX_BUFFERED_PACKETS, SpeechOutput, SpeechOutputEvent};
    use crate::{
        audio::PcmF32Mono,
        providers::{TtsError, TtsProvider, TtsWorker},
        workers::{TtsWorkerRuntime, WorkerRuntimeConfig},
    };
    use std::{
        sync::{Arc, Mutex, atomic::AtomicBool, mpsc},
        time::Duration,
    };

    struct LargeChunkProvider(Arc<Mutex<mpsc::Receiver<()>>>);
    struct LargeChunkWorker(Arc<Mutex<mpsc::Receiver<()>>>);
    impl TtsProvider for LargeChunkProvider {
        fn adapter(&self) -> &'static str {
            "large-chunk"
        }
        fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
            Ok(Box::new(LargeChunkWorker(Arc::clone(&self.0))))
        }
    }
    impl TtsWorker for LargeChunkWorker {
        fn synthesize(
            &mut self,
            _: &str,
            _: &AtomicBool,
            on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
        ) -> Result<(), TtsError> {
            on_pcm(PcmF32Mono::new(
                vec![0.1; 2_880 * MAX_BUFFERED_PACKETS],
                48_000,
            ))?;
            self.0.lock().unwrap().recv().unwrap();
            Ok(())
        }
        fn reset(&mut self) -> Result<(), TtsError> {
            Ok(())
        }
    }

    let (release_tx, release_rx) = mpsc::channel();
    let provider: Arc<dyn TtsProvider> =
        Arc::new(LargeChunkProvider(Arc::new(Mutex::new(release_rx))));
    let runtime = Arc::new(TtsWorkerRuntime::new(
        Arc::clone(&provider),
        WorkerRuntimeConfig {
            max_workers: 1,
            command_capacity: 4,
            final_timeout: Duration::from_secs(2),
            cleanup_grace: Duration::from_secs(1),
        },
    ));
    let mut output =
        SpeechOutput::with_worker(provider, runtime, SpeechOutputConfig::default()).unwrap();
    output
        .push_delta("Một câu đủ dài để kiểm tra bộ đệm đầu lượt.")
        .unwrap();
    assert!(matches!(
        output.poll().unwrap(),
        Some(SpeechOutputEvent::SegmentReady { .. })
    ));
    let mut started = false;
    for _ in 0..10_000 {
        if matches!(output.poll().unwrap(), Some(SpeechOutputEvent::Started)) {
            started = true;
            break;
        }
        std::thread::yield_now();
    }
    assert!(
        started,
        "bounded long segment must start without waiting for worker completion"
    );
    assert!(output.active_worker.is_some());
    assert!(output.packets.len() >= MAX_BUFFERED_PACKETS);
    release_tx.send(()).unwrap();
}

#[test]
fn first_five_packets_are_sent_before_realtime_pacing() {
    use super::{SpeechOutput, SpeechOutputEvent};
    use crate::{
        audio::PcmF32Mono,
        providers::{TtsError, TtsProvider},
    };
    use std::sync::Arc;

    struct TenFrames;
    impl TtsProvider for TenFrames {
        fn adapter(&self) -> &'static str {
            "ten-frames"
        }
        fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
            Ok(PcmF32Mono::new(vec![0.1; 2_880 * 10], 48_000))
        }
    }
    let mut output =
        SpeechOutput::with_config(Arc::new(TenFrames), SpeechOutputConfig::default()).unwrap();
    output.push_delta("Một câu đủ dài.").unwrap();
    output.finish_input().unwrap();
    assert!(matches!(
        output.poll().unwrap(),
        Some(SpeechOutputEvent::SegmentReady { .. })
    ));
    assert!(matches!(
        output.poll().unwrap(),
        Some(SpeechOutputEvent::Started)
    ));
    for packet in 1..=5 {
        assert!(
            matches!(
                output.poll().unwrap(),
                Some(SpeechOutputEvent::AudioPacket(_))
            ),
            "packet {packet} should prebuffer immediately"
        );
    }
    assert!(
        output.poll().unwrap().is_none(),
        "sixth packet must wait for pacing"
    );
    let origin = std::time::Instant::now() - std::time::Duration::from_millis(61);
    output.playback_origin = Some(origin);
    assert!(matches!(
        output.poll().unwrap(),
        Some(SpeechOutputEvent::AudioPacket(_))
    ));
    assert_eq!(
        output.packet_send_deadline(),
        Some(origin + std::time::Duration::from_millis(120)),
        "late polls must retain the absolute cadence"
    );
}

#[test]
fn paced_deadlines_are_anchored_to_first_packet_even_if_prebuffer_is_delayed() {
    use super::SpeechOutput;
    use crate::providers::tts::UnavailableTts;
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    let mut output =
        SpeechOutput::with_config(Arc::new(UnavailableTts), SpeechOutputConfig::default()).unwrap();
    let first_packet_at = Instant::now();
    output.playback_origin = Some(first_packet_at);
    output.packets_sent = 5;
    assert_eq!(
        output.packet_send_deadline(),
        Some(first_packet_at + Duration::from_millis(60))
    );
    output.packets_sent = 6;
    assert_eq!(
        output.packet_send_deadline(),
        Some(first_packet_at + Duration::from_millis(120))
    );
}

#[test]
fn next_segment_starts_while_previous_audio_is_queued() {
    use super::{SpeechOutput, SpeechOutputEvent};
    use crate::{
        audio::PcmF32Mono,
        providers::{TtsError, TtsProvider},
    };
    use std::sync::{Arc, Mutex};

    struct RecordingTts(Arc<Mutex<Vec<String>>>);
    impl TtsProvider for RecordingTts {
        fn adapter(&self) -> &'static str {
            "recording"
        }
        fn synthesize(&self, text: &str) -> Result<PcmF32Mono, TtsError> {
            self.0.lock().unwrap().push(text.into());
            Ok(PcmF32Mono::new(vec![0.1; 2_880 * 10], 48_000))
        }
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut output = SpeechOutput::with_config(
        Arc::new(RecordingTts(Arc::clone(&calls))),
        SpeechOutputConfig::default(),
    )
    .unwrap();
    output.push_delta("Câu thứ nhất. Câu thứ hai.").unwrap();
    assert!(matches!(
        output.poll().unwrap(),
        Some(SpeechOutputEvent::SegmentReady { .. })
    ));
    assert!(output.poll().unwrap().is_none());
    assert!(!output.packets.is_empty());
    assert!(
        matches!(
            output.poll().unwrap(),
            Some(SpeechOutputEvent::SegmentReady { .. })
        ),
        "next segment must be announced before first audio queue drains"
    );
    assert!(matches!(
        output.poll().unwrap(),
        Some(SpeechOutputEvent::Started)
    ));
    assert!(matches!(
        output.poll().unwrap(),
        Some(SpeechOutputEvent::AudioPacket(_))
    ));
    assert_eq!(
        calls.lock().unwrap().len(),
        2,
        "second synthesis must overlap first playback"
    );
}

#[test]
fn prebuffered_audio_does_not_drain_before_nominal_playback_ends() {
    use super::{SpeechOutput, SpeechOutputEvent};
    use crate::{
        audio::PcmF32Mono,
        providers::{TtsError, TtsProvider},
    };
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    struct ShortTts;
    impl TtsProvider for ShortTts {
        fn adapter(&self) -> &'static str {
            "short"
        }
        fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
            Ok(PcmF32Mono::new(vec![0.1; 2_880 * 2], 48_000))
        }
    }
    let mut output =
        SpeechOutput::with_config(Arc::new(ShortTts), SpeechOutputConfig::default()).unwrap();
    output.push_delta("Một câu ngắn.").unwrap();
    output.finish_input().unwrap();
    assert!(matches!(
        output.poll().unwrap(),
        Some(SpeechOutputEvent::SegmentReady { .. })
    ));
    assert!(matches!(
        output.poll().unwrap(),
        Some(SpeechOutputEvent::Started)
    ));
    assert!(matches!(
        output.poll().unwrap(),
        Some(SpeechOutputEvent::AudioPacket(_))
    ));
    assert!(matches!(
        output.poll().unwrap(),
        Some(SpeechOutputEvent::AudioPacket(_))
    ));
    assert!(output.playback_end_deadline.unwrap() > Instant::now() + Duration::from_millis(60));
    assert!(
        output.poll().unwrap().is_none(),
        "tts:stop must wait for buffered playback"
    );
    output.playback_end_deadline = Some(Instant::now() - Duration::from_millis(1));
    assert!(matches!(
        output.poll().unwrap(),
        Some(SpeechOutputEvent::Drained)
    ));
}

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
fn keeps_date_and_time_separators_for_zerotts_normalization() {
    let text = sanitize_tts_text("Hôm nay là Tuesday, 23/09/2026 12:54:56 UTC.");
    assert_eq!(text, "Hôm nay là Tuesday, 23/09/2026 12:54:56 UTC.");
}

#[test]
fn removes_json_command_across_deltas_before_sentence_delivery() {
    let mut filter = super::JsonFilter::default();
    let mut segmenter = SentenceSegmenter::new(SpeechOutputConfig::default());
    let mut segments = Vec::new();
    for delta in [
        "{\"cmd\": \"date '+%A, %d/%m/",
        "%Y %H:%M:%S %Z'\"}Hôm nay là Tuesday, ",
        "23/09/2026 12:54:56 UTC.",
    ] {
        segments.extend(segmenter.push(&filter.push(delta)));
    }
    segments.extend(segmenter.push(&filter.finish()));
    segments.extend(segmenter.finish());
    assert_eq!(segments, ["Hôm nay là Tuesday, 23/09/2026 12:54:56 UTC."]);
}

#[test]
fn json_filter_handles_escaped_braces_and_keeps_non_json_braces() {
    let mut filter = super::JsonFilter::default();
    assert_eq!(
        filter.push("Xin {\"note\": \"brace } and \\\"quote\\\"\"}chào {bạn}."),
        "Xin chào {bạn}."
    );
    assert_eq!(filter.push("Mời [1, {\"cmd\": \"skip\"}] bạn."), "Mời bạn.");
    assert_eq!(filter.finish(), "");
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

    output.cancel();
    output.push_delta("{\"cmd\": \"date '+%A, %d/%m/").unwrap();
    output
        .push_delta("%Y %H:%M:%S %Z'\"}Hôm nay là Tuesday, 23/09/2026 12:54:56 UTC.")
        .unwrap();
    output.finish_input().unwrap();
    assert!(matches!(
        output.poll().unwrap(),
        Some(SpeechOutputEvent::SegmentReady { text })
            if text == "Hôm nay là Tuesday, 23/09/2026 12:54:56 UTC."
    ));
    assert!(matches!(
        output.poll().unwrap(),
        Some(SpeechOutputEvent::Started)
    ));
    assert_eq!(
        *inputs.lock().unwrap(),
        ["Xin chào!", "Hôm nay là Tuesday, 23/09/2026 12:54:56 UTC."]
    );
}

#[test]
fn unspeakable_response_fails_instead_of_waiting_forever() {
    use super::{SpeechOutput, SpeechOutputError};
    use crate::providers::tts::UnavailableTts;
    use std::sync::Arc;

    let mut output =
        SpeechOutput::with_config(Arc::new(UnavailableTts), SpeechOutputConfig::default()).unwrap();
    output.push_delta("😊🚀").unwrap();
    assert_eq!(output.finish_input(), Err(SpeechOutputError::Synthesis));
}
