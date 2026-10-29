use super::*;
use crate::audio::PcmF32Mono;

impl ProviderDiagnosticService {
    /// Speaker extraction is managed-only. The native request owns its exact lease and capacity
    /// until completion even if the bounded response wait expires.
    pub async fn execute_speaker(
        &self,
        key: &str,
        expected_revision: i64,
        pcm: PcmF32Mono,
    ) -> Result<serde_json::Value, ProviderDiagnosticRequestError> {
        let snapshot = self.request_snapshot(key, "speaker").await?;
        if snapshot.revision != expected_revision {
            return Err(ProviderDiagnosticRequestError::RevisionConflict);
        }
        let config: crate::config::CampPlusConfig = serde_json::from_str(&snapshot.config_json)
            .map_err(|_| ProviderDiagnosticRequestError::InvalidInput)?;
        if !config.valid() {
            return Err(ProviderDiagnosticRequestError::InvalidInput);
        }
        let manager = self
            .manager
            .as_ref()
            .ok_or(ProviderDiagnosticRequestError::SpeakerManagerRequired)?;
        let samples = pcm.samples();
        let rms = (samples.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>()
            / samples.len().max(1) as f64)
            .sqrt();
        let clipping_fraction = samples.iter().filter(|v| v.abs() >= 0.999).count() as f64
            / samples.len().max(1) as f64;
        let duration_ms = samples.len() * 1000 / 16_000;
        if pcm.sample_rate_hz() != 16_000
            || !(1000..=12_000).contains(&duration_ms)
            || rms < 0.001
            || clipping_fraction > 0.05
        {
            return Err(ProviderDiagnosticRequestError::InvalidInput);
        }
        // ponytail: diagnostic uses the first bounded window; shared calibrated VAD/window
        // selection belongs to enrollment/Observe quality integration.
        let window_samples = usize::try_from(config.max_window_ms)
            .map_err(|_| ProviderDiagnosticRequestError::InvalidInput)?
            * 16;
        let pcm = PcmF32Mono::new(
            pcm.samples()[..pcm.samples().len().min(window_samples)].to_vec(),
            16_000,
        );
        let _diagnostic = self.limiter.try_acquire()?;
        let lease = manager.acquire(snapshot.clone()).await?;
        let runtime = lease
            .runtimes()
            .and_then(|catalog| catalog.speaker(key))
            .ok_or(ProviderDiagnosticError::Unavailable)?;
        let dimension = runtime.dimension();
        let embedding_space_id = runtime.embedding_space_id().to_owned();
        tokio::time::timeout(self.execution_timeout, runtime.extract(pcm, lease))
            .await
            .map_err(|_| ProviderDiagnosticError::Timeout)?
            .map_err(map_speaker_error)?;
        let current = self.request_snapshot(key, "speaker").await?;
        if current.id != snapshot.id || current.revision != snapshot.revision {
            return Err(ProviderDiagnosticRequestError::RevisionConflict);
        }
        Ok(
            serde_json::json!({"provider_key": snapshot.key, "type":"speaker", "status":"success", "quality":{"duration_ms":duration_ms,"rms":rms,"clipping_fraction":clipping_fraction}, "provenance":{"embedding_space_id":embedding_space_id,"dimension":dimension,"model_revision":crate::providers::speaker::assets::MODEL_REVISION,"preprocessing":"pcm16-mono16k-v1"}, "runtime":{"tested_provider_id":snapshot.id,"tested_revision":snapshot.revision,"runtime_matches_desired":true,"requires_restart":false}}),
        )
    }

    /// One enrollment embedding. The same managed-only bounded-worker path as
    /// [`Self::execute_speaker`], but it returns the vector instead of a diagnostic summary. The
    /// caller has already run the quality gate and selected the window, so this boundary owns only
    /// runtime lookup, admission, timeout, and the post-run revision check.
    pub async fn extract_speaker_embedding(
        &self,
        key: &str,
        expected_revision: i64,
        pcm: PcmF32Mono,
    ) -> Result<SpeakerEmbeddingResult, ProviderDiagnosticRequestError> {
        let snapshot = self.request_snapshot(key, "speaker").await?;
        if snapshot.revision != expected_revision {
            return Err(ProviderDiagnosticRequestError::RevisionConflict);
        }
        let config: crate::config::CampPlusConfig = serde_json::from_str(&snapshot.config_json)
            .map_err(|_| ProviderDiagnosticRequestError::InvalidInput)?;
        if !config.valid() {
            return Err(ProviderDiagnosticRequestError::InvalidInput);
        }
        let manager = self
            .manager
            .as_ref()
            .ok_or(ProviderDiagnosticRequestError::SpeakerManagerRequired)?;
        let _diagnostic = self.limiter.try_acquire()?;
        let lease = manager.acquire(snapshot.clone()).await?;
        let runtime = lease
            .runtimes()
            .and_then(|catalog| catalog.speaker(key))
            .ok_or(ProviderDiagnosticError::Unavailable)?;
        let dimension = runtime.dimension();
        let embedding_space_id = runtime.embedding_space_id().to_owned();
        let embedding = tokio::time::timeout(self.execution_timeout, runtime.extract(pcm, lease))
            .await
            .map_err(|_| ProviderDiagnosticError::Timeout)?
            .map_err(map_speaker_error)?;
        let current = self.request_snapshot(key, "speaker").await?;
        if current.id != snapshot.id || current.revision != snapshot.revision {
            return Err(ProviderDiagnosticRequestError::RevisionConflict);
        }
        Ok(SpeakerEmbeddingResult {
            embedding,
            embedding_space_id,
            dimension,
        })
    }
}

fn map_speaker_error(
    err: crate::providers::speaker::SpeakerError,
) -> ProviderDiagnosticRequestError {
    match err {
        crate::providers::speaker::SpeakerError::InvalidInput => {
            ProviderDiagnosticRequestError::InvalidInput
        }
        crate::providers::speaker::SpeakerError::Busy => ProviderDiagnosticError::Busy.into(),
        crate::providers::speaker::SpeakerError::InvalidEmbedding => {
            ProviderDiagnosticError::InvalidResponse.into()
        }
        crate::providers::speaker::SpeakerError::Unavailable => {
            ProviderDiagnosticError::Unavailable.into()
        }
    }
}
