use super::*;
use crate::providers::{TtsError, TtsProvider};
use std::time::Duration;

struct ControlledTts {
    chunks: Mutex<mpsc::Receiver<usize>>,
    emitted: mpsc::Sender<()>,
}
impl TtsProvider for ControlledTts {
    fn adapter(&self) -> &'static str {
        "controlled"
    }
    fn synthesize_stream(
        &self,
        _: &str,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        while let Ok(samples) = self.chunks.lock().unwrap().recv() {
            on_pcm(PcmF32Mono::new(vec![0.1; samples], 48_000))?;
            self.emitted.send(()).unwrap();
        }
        Ok(())
    }
}
fn fixture() -> (TtsWorkerRuntime, mpsc::Sender<usize>, mpsc::Receiver<()>) {
    let (chunks_tx, chunks_rx) = mpsc::channel();
    let (emitted_tx, emitted_rx) = mpsc::channel();
    let runtime = TtsWorkerRuntime::new(
        Arc::new(ControlledTts {
            chunks: Mutex::new(chunks_rx),
            emitted: emitted_tx,
        }),
        WorkerRuntimeConfig {
            max_workers: 1,
            final_timeout: Duration::from_secs(15),
            cleanup_grace: Duration::from_secs(5),
            ..WorkerRuntimeConfig::default()
        },
    );
    (runtime, chunks_tx, emitted_rx)
}
fn emit(chunks: &mpsc::Sender<usize>, emitted: &mpsc::Receiver<()>, samples: usize) {
    chunks.send(samples).unwrap();
    emitted.recv_timeout(Duration::from_secs(2)).unwrap();
}

#[tokio::test(start_paused = true)]
async fn voice_stall_times_out_after_last_pcm_progress() {
    let (runtime, chunks, emitted) = fixture();
    let lease = runtime.start("fixture".into()).unwrap();
    tokio::time::advance(Duration::from_secs(10)).await;
    emit(&chunks, &emitted, 96);
    assert!(matches!(
        runtime.poll(lease).unwrap(),
        Some(TtsWorkerEvent::Pcm(_))
    ));
    tokio::time::advance(Duration::from_secs(14)).await;
    assert!(runtime.poll(lease).unwrap().is_none());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(
        runtime.poll(lease).unwrap(),
        Some(TtsWorkerEvent::TimedOut)
    ));
    tokio::time::advance(Duration::from_secs(5)).await;
    assert!(matches!(
        runtime.poll(lease).unwrap(),
        Some(TtsWorkerEvent::CleanupTimedOut)
    ));
}

#[tokio::test(start_paused = true)]
async fn voice_without_first_pcm_still_times_out_from_acceptance() {
    let (runtime, _chunks, _emitted) = fixture();
    let lease = runtime.start("fixture".into()).unwrap();
    tokio::time::advance(Duration::from_secs(15)).await;
    assert!(matches!(
        runtime.poll(lease).unwrap(),
        Some(TtsWorkerEvent::TimedOut)
    ));
}

#[tokio::test(start_paused = true)]
async fn empty_pcm_does_not_extend_voice_timeout() {
    let (runtime, chunks, emitted) = fixture();
    let lease = runtime.start("fixture".into()).unwrap();
    tokio::time::advance(Duration::from_secs(10)).await;
    emit(&chunks, &emitted, 0);
    assert!(matches!(
        runtime.poll(lease).unwrap(),
        Some(TtsWorkerEvent::Pcm(_))
    ));
    tokio::time::advance(Duration::from_secs(5)).await;
    assert!(matches!(
        runtime.poll(lease).unwrap(),
        Some(TtsWorkerEvent::TimedOut)
    ));
}

#[tokio::test(start_paused = true)]
async fn diagnostic_pcm_does_not_extend_absolute_timeout() {
    let (runtime, chunks, emitted) = fixture();
    let lease = runtime
        .start_diagnostic(TtsDiagnosticRequest {
            text: "fixture".into(),
            voice: None,
            language: None,
        })
        .unwrap();
    tokio::time::advance(Duration::from_secs(10)).await;
    emit(&chunks, &emitted, 96);
    assert!(matches!(
        runtime.poll(lease).unwrap(),
        Some(TtsWorkerEvent::Pcm(_))
    ));
    tokio::time::advance(Duration::from_secs(5)).await;
    assert!(matches!(
        runtime.poll(lease).unwrap(),
        Some(TtsWorkerEvent::TimedOut)
    ));
}

#[tokio::test(start_paused = true)]
async fn queued_voice_pcm_does_not_charge_backpressure_as_a_stall() {
    let (runtime, chunks, emitted) = fixture();
    let lease = runtime.start("fixture".into()).unwrap();
    emit(&chunks, &emitted, 96);
    // SpeechOutput stops polling at its audio high-water mark while playback drains.
    tokio::time::advance(Duration::from_secs(30)).await;
    assert!(matches!(
        runtime.poll(lease).unwrap(),
        Some(TtsWorkerEvent::Pcm(_))
    ));
    assert!(runtime.poll(lease).unwrap().is_none());
    tokio::time::advance(Duration::from_secs(15)).await;
    assert!(matches!(
        runtime.poll(lease).unwrap(),
        Some(TtsWorkerEvent::TimedOut)
    ));
}
