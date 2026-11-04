//! Bounded worker owns native mutable state and terminal resource obligations.
use crate::providers::speaker::{SpeakerError, SpeakerProvider};
use crate::{
    audio::PcmF32Mono,
    services::provider_runtime::ResourceLease,
    workers::{ProviderCapacityPermit, ProviderRuntimeAdmission, ProviderWorkloadClass},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
/// Shortest window the resident extractor accepts (1 s at the canonical 16 kHz).
pub const MIN_WINDOW_SAMPLES: usize = 16_000;
/// Longest window the resident extractor accepts (6 s at the canonical 16 kHz).
pub const MAX_WINDOW_SAMPLES: usize = 96_000;

struct Request {
    pcm: PcmF32Mono,
    reply: tokio::sync::oneshot::Sender<Result<Vec<f32>, SpeakerError>>,
    _lease: Option<ResourceLease>,
    _capacity: ProviderCapacityPermit,
    _slot: tokio::sync::OwnedSemaphorePermit,
}

/// One resident extractor, a bounded queue, and native exit acknowledgement. Requests retain
/// resource/capacity ownership even if their HTTP or enrollment waiter is cancelled.
#[derive(Clone)]
pub struct SpeakerRuntime {
    sender: Arc<Mutex<Option<mpsc::SyncSender<Request>>>>,
    exited: Arc<AtomicBool>,
    healthy: Arc<AtomicBool>,
    slot: Arc<tokio::sync::Semaphore>,
    admission: ProviderRuntimeAdmission,
    dimension: usize,
    embedding_space_id: Arc<str>,
}
impl SpeakerRuntime {
    pub fn new(
        mut provider: Box<dyn SpeakerProvider>,
        admission: ProviderRuntimeAdmission,
    ) -> Result<Self, SpeakerError> {
        let dimension = provider.dimension();
        // Bounded readiness touches the exact retained extractor; this is not accuracy evidence.
        let warmup = PcmF32Mono::new(
            (0..16_000)
                .map(|i| ((i as f32) * 0.03).sin() * 0.01)
                .collect(),
            16_000,
        );
        validate_embedding(provider.extract(&warmup)?, dimension)?;
        if !(1..=4096).contains(&dimension) {
            return Err(SpeakerError::InvalidEmbedding);
        }
        use sha2::{Digest, Sha256};
        let contract = serde_json::json!({"model_revision":crate::providers::speaker::assets::MODEL_REVISION,"extractor":"sherpa-onnx-1.13.8-campplus","preprocessing":"pcm16-mono16k-v1","dimension":dimension});
        let digest = Sha256::digest(
            serde_json::to_vec(&contract).map_err(|_| SpeakerError::InvalidEmbedding)?,
        );
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        let embedding_space_id: Arc<str> = format!("speaker:{hex}").into();
        let (sender, receiver) = mpsc::sync_channel::<Request>(1);
        let exited = Arc::new(AtomicBool::new(false));
        let exit = exited.clone();
        let healthy = Arc::new(AtomicBool::new(true));
        let health = healthy.clone();
        std::thread::Builder::new()
            .name("speaker-native".into())
            .spawn(move || {
                // Native destructors must finish before the manager can reuse memory.
                struct Native {
                    provider: Option<Box<dyn SpeakerProvider>>,
                    exited: Arc<AtomicBool>,
                    healthy: Arc<AtomicBool>,
                }
                impl Drop for Native {
                    fn drop(&mut self) {
                        self.healthy.store(false, Ordering::Release);
                        drop(self.provider.take());
                        self.exited.store(true, Ordering::Release);
                    }
                }
                let mut native = Native {
                    provider: Some(provider),
                    exited: exit,
                    healthy: health,
                };
                while let Ok(request) = receiver.recv() {
                    let result = native
                        .provider
                        .as_mut()
                        .expect("resident speaker")
                        .extract(&request.pcm)
                        .and_then(|embedding| validate_embedding(embedding, dimension));
                    let _ = request.reply.send(result);
                }
            })
            .map_err(|_| SpeakerError::Unavailable)?;
        Ok(Self {
            sender: Arc::new(Mutex::new(Some(sender))),
            exited,
            healthy,
            slot: Arc::new(tokio::sync::Semaphore::new(1)),
            admission,
            dimension,
            embedding_space_id,
        })
    }
    pub fn health_flags(&self) -> Vec<Arc<AtomicBool>> {
        vec![self.healthy.clone()]
    }
    pub fn native_pending(&self) -> bool {
        self.slot.available_permits() == 0
    }
    pub fn dimension(&self) -> usize {
        self.dimension
    }
    /// Shortest window [`Self::extract`] accepts. A shorter utterance is not a runtime failure.
    pub fn min_window_samples(&self) -> usize {
        MIN_WINDOW_SAMPLES
    }
    pub fn embedding_space_id(&self) -> &str {
        &self.embedding_space_id
    }
    pub fn logical_view(&self, admission: ProviderRuntimeAdmission) -> Self {
        Self {
            admission,
            ..self.clone()
        }
    }
    pub fn shutdown_acknowledged(&self) -> bool {
        self.sender.lock().expect("speaker sender poisoned").take();
        self.exited.load(Ordering::Acquire)
    }
    pub async fn extract(
        &self,
        pcm: PcmF32Mono,
        lease: ResourceLease,
    ) -> Result<Vec<f32>, SpeakerError> {
        self.extract_inner(pcm, Some(lease)).await
    }

    /// The built-in runtime is owned by AppState for the entire process, not by a
    /// database provider lease. The worker request still owns its capacity/slot.
    pub async fn extract_builtin(&self, pcm: PcmF32Mono) -> Result<Vec<f32>, SpeakerError> {
        self.extract_inner(pcm, None).await
    }

    async fn extract_inner(
        &self,
        pcm: PcmF32Mono,
        lease: Option<ResourceLease>,
    ) -> Result<Vec<f32>, SpeakerError> {
        if pcm.sample_rate_hz() != 16_000
            || !(MIN_WINDOW_SAMPLES..=MAX_WINDOW_SAMPLES).contains(&pcm.samples().len())
            || pcm
                .samples()
                .iter()
                .any(|x| !x.is_finite() || x.abs() > 1.0)
        {
            return Err(SpeakerError::InvalidInput);
        }
        let slot = self
            .slot
            .clone()
            .try_acquire_owned()
            .map_err(|_| SpeakerError::Busy)?;
        // Speaker has one physical slot, including diagnostics. It cannot reserve that only
        // slot for Voice while rejecting enrollment and diagnostics permanently.
        let capacity = self
            .admission
            .try_admit(ProviderWorkloadClass::Voice)
            .map_err(|_| SpeakerError::Busy)?;
        let (reply, response) = tokio::sync::oneshot::channel();
        let request = Request {
            pcm,
            reply,
            _lease: lease,
            _capacity: capacity,
            _slot: slot,
        };
        self.sender
            .lock()
            .expect("speaker sender poisoned")
            .as_ref()
            .ok_or(SpeakerError::Unavailable)?
            .try_send(request)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => SpeakerError::Busy,
                mpsc::TrySendError::Disconnected(_) => SpeakerError::Unavailable,
            })?;
        response.await.map_err(|_| SpeakerError::Unavailable)?
    }
}
fn validate_embedding(embedding: Vec<f32>, dimension: usize) -> Result<Vec<f32>, SpeakerError> {
    if embedding.len() != dimension || embedding.iter().any(|v| !v.is_finite()) {
        return Err(SpeakerError::InvalidEmbedding);
    }
    let norm = embedding
        .iter()
        .map(|v| f64::from(*v).powi(2))
        .sum::<f64>()
        .sqrt();
    if !norm.is_finite() || norm <= 1e-12 {
        return Err(SpeakerError::InvalidEmbedding);
    }
    Ok(embedding)
}
