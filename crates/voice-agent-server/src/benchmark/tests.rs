use super::{TtsBenchmarkMode, run_tts_benchmark};
use crate::{
    audio::PcmF32Mono,
    providers::{TtsError, TtsWorker},
};
use std::sync::atomic::AtomicBool;

struct Worker {
    syntheses: usize,
    resets: usize,
}
impl TtsWorker for Worker {
    fn synthesize(
        &mut self,
        _: &str,
        _: &AtomicBool,
        callback: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        self.syntheses += 1;
        callback(PcmF32Mono::new(vec![0.25; 2880], 48000))
    }
    fn reset(&mut self) -> Result<(), TtsError> {
        self.resets += 1;
        Ok(())
    }
}

#[test]
fn delivery_excludes_warmup_and_reports_canonical_packets() {
    let mut worker = Worker {
        syntheses: 0,
        resets: 0,
    };
    let report = run_tts_benchmark(&mut worker, TtsBenchmarkMode::Delivery, 1, 2).unwrap();
    assert_eq!(worker.syntheses, 3);
    assert_eq!(worker.resets, 3);
    assert_eq!(report.samples.len(), 2);
    assert!(
        report
            .samples
            .iter()
            .all(|s| s.opus_bytes.is_some_and(|bytes| bytes > 0))
    );
    assert!(report.delivery_total_ms.is_some());
}

#[test]
fn invalid_pcm_is_rejected() {
    struct Invalid;
    impl TtsWorker for Invalid {
        fn synthesize(
            &mut self,
            _: &str,
            _: &AtomicBool,
            callback: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
        ) -> Result<(), TtsError> {
            callback(PcmF32Mono::new(vec![f32::NAN], 48000))
        }
        fn reset(&mut self) -> Result<(), TtsError> {
            Ok(())
        }
    }
    let mut worker = Invalid;
    assert!(run_tts_benchmark(&mut worker, TtsBenchmarkMode::Provider, 0, 1).is_err());
}
