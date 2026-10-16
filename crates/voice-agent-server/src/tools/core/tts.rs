//! Deep Core TTS operation: exact version pin, native worker ownership and WAV artifact.
use crate::{
    providers::config_catalog::ProviderOrigin,
    services::{
        background::{
            JobEngine,
            plugins::{ArtifactData, Invocation, InvokeResult, PluginError},
        },
        provider_runtime::{ResourceLease, ProviderRuntimeManager},
    },
    workers::{TtsLease, TtsWorkerEvent, TtsWorkerRuntime},
};
use std::{path::PathBuf, sync::{Arc,Mutex}, time::{Duration,Instant}};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

const MAX_TEXT_CHARS: usize = 3000;
const MAX_AUDIO_SECONDS: u64 = 180;
const MIN_SAMPLE_RATE: u32 = 8_000;
const MAX_SAMPLE_RATE: u32 = 192_000;

/// An aborted Future must not release the versioned provider lease while native
/// inference is still active. The TTS worker owns its Background permit.
struct NativeAttempt<'a> {
    runtime: Arc<TtsWorkerRuntime>,
    resource: Option<ResourceLease>,
    active: Option<TtsLease>,
    quarantine: &'a Mutex<Vec<ResourceLease>>,
}
impl NativeAttempt<'_> {
    async fn stop_and_ack(&mut self) -> bool {
        let Some(id) = self.active else { return true };
        if self.runtime.cancel(id).is_err() { return false }
        let deadline = Instant::now() + self.runtime.cleanup_grace();
        while Instant::now() < deadline {
            match self.runtime.poll(id) {
                Ok(Some(TtsWorkerEvent::Finished | TtsWorkerEvent::Failed | TtsWorkerEvent::Cancelled)) => {
                    self.active = None;
                    return true
                }
                Ok(Some(TtsWorkerEvent::CleanupTimedOut)) | Err(_) => return false,
                _ => tokio::time::sleep(Duration::from_millis(5)).await,
            }
        }
        false
    }
}
impl Drop for NativeAttempt<'_> {
    fn drop(&mut self) {
        if let Some(active) = self.active.take() {
            let _ = self.runtime.cancel_and_detach(active);
            if let Some(resource) = self.resource.take() {
                self.quarantine.lock().expect("TTS quarantine poisoned").push(resource);
            }
        }
    }
}
struct OutputWorkspace {
    path: PathBuf,
    retain: bool,
}
impl Drop for OutputWorkspace {
    fn drop(&mut self) {
        if !self.retain {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn wav_header(rate: u32, data_bytes: u32) -> Option<[u8;44]> {
    let mut bytes=[0u8;44];
    let byte_rate=rate.checked_mul(2)?;
    let riff_size=data_bytes.checked_add(36)?;
    bytes[0..4].copy_from_slice(b"RIFF");
    bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
    bytes[8..16].copy_from_slice(b"WAVEfmt ");
    bytes[16..20].copy_from_slice(&16u32.to_le_bytes());
    bytes[20..22].copy_from_slice(&1u16.to_le_bytes());
    bytes[22..24].copy_from_slice(&1u16.to_le_bytes());
    bytes[24..28].copy_from_slice(&rate.to_le_bytes());
    bytes[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    bytes[32..34].copy_from_slice(&2u16.to_le_bytes());
    bytes[34..36].copy_from_slice(&16u16.to_le_bytes());
    bytes[36..40].copy_from_slice(b"data");
    bytes[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    Some(bytes)
}

pub(crate) async fn synthesize(
    engine: &JobEngine,
    request: Invocation<'_>,
) -> Result<InvokeResult, PluginError> {
    let text=request.arguments["text"].as_str().ok_or(PluginError::ApplicationError)?;
    let chars=text.chars().count();
    if chars==0 || chars>MAX_TEXT_CHARS || text.trim().is_empty() {
        return Err(PluginError::ApplicationError);
    }
    let configured=request.settings.as_ref().and_then(|v|v["provider_ref"].as_str())
        .ok_or(PluginError::PermissionRequired)?;
    let (snapshot,origin,key)=engine.database
        .pinned_tts_provider(&engine.config,request.job_id).await
        .map_err(PluginError::JobRejected)?;
    let actual=match origin {
        ProviderOrigin::Config => format!("config:tts:{key}"),
        ProviderOrigin::Database => key.clone(),
    };
    if configured!=actual { return Err(PluginError::PermissionRequired) }
    let manager: &Arc<ProviderRuntimeManager>=engine.provider_manager.as_ref()
        .ok_or(PluginError::WorkerCapacityUnavailable)?;
    let deadline=tokio::time::Instant::now()+request.timeout;
    let resource=tokio::select! {
        _ = request.cancellation.cancelled() => return Err(PluginError::JobRejected(crate::services::background::JobError::Cancelled)),
        outcome = async {
            match origin {
                ProviderOrigin::Config => manager.acquire_deployment_until(snapshot,deadline).await,
                ProviderOrigin::Database => manager.acquire_until(snapshot,deadline).await,
            }
        } => outcome.map_err(|_| PluginError::WorkerCapacityUnavailable)?,
    };
    let runtime=resource.runtimes()
        .ok_or(PluginError::WorkerCapacityUnavailable)?
        .tts(&key).map_err(|_|PluginError::WorkerCapacityUnavailable)?;
    if !runtime.has_background_capacity() {return Err(PluginError::WorkerCapacityUnavailable)}
    // Recheck the immutable job pin after waiting for a model to become ready.
    engine.database.pinned_tts_provider(&engine.config,request.job_id)
        .await.map_err(PluginError::JobRejected)?;

    let data_root=engine.database.durable_path()
        .and_then(std::path::Path::parent)
        .ok_or(PluginError::StorageUnavailable)?;
    let workspace=data_root.join("plugins").join("workspaces")
        .join(request.job_id).join(request.attempt_id);
    tokio::fs::create_dir_all(&workspace).await
        .map_err(|_|PluginError::StorageUnavailable)?;
    let mut cleanup=OutputWorkspace{path:workspace.clone(),retain:false};
    let path=workspace.join("speech.wav");
    let mut file=tokio::fs::OpenOptions::new().write(true).create_new(true)
        .open(&path).await.map_err(|_|PluginError::StorageUnavailable)?;
    file.write_all(&[0u8;44]).await.map_err(|_|PluginError::StorageUnavailable)?;

    let id=runtime.start_background(text.to_owned())
        .map_err(|_|PluginError::WorkerCapacityUnavailable)?;
    let mut native=NativeAttempt{runtime,resource:Some(resource),active:Some(id),
        quarantine:&engine.tts_quarantine};
    let mut sample_rate:Option<u32>=None;
    let mut samples=0u64;
    let mut failure=None;
    loop {
        if request.cancellation.is_cancelled() {
            failure=Some(PluginError::JobRejected(crate::services::background::JobError::Cancelled));
            break;
        }
        if tokio::time::Instant::now()>=deadline {
            failure=Some(PluginError::WorkerTimeout);
            break;
        }
        match native.runtime.poll(id) {
            Ok(Some(TtsWorkerEvent::Pcm(pcm))) => {
                let rate=pcm.sample_rate_hz();
                if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&rate)
                    || sample_rate.is_some_and(|current|current!=rate)
                    || pcm.samples().iter().any(|x|!x.is_finite())
                    || samples.saturating_add(pcm.samples().len() as u64)>rate as u64*MAX_AUDIO_SECONDS
                    || (samples.saturating_add(pcm.samples().len() as u64))*2+44
                         >engine.config.background.limits.artifact_bytes_per_job {
                    failure=Some(PluginError::ApplicationError);
                    break;
                }
                sample_rate=Some(rate);
                samples+=pcm.samples().len() as u64;
                let mut encoded=Vec::with_capacity(pcm.samples().len()*2);
                for sample in pcm.samples() {
                    let value=(sample.clamp(-1.0,1.0)*f32::from(i16::MAX)).round() as i16;
                    encoded.extend_from_slice(&value.to_le_bytes());
                }
                if file.write_all(&encoded).await.is_err() {
                    failure=Some(PluginError::StorageUnavailable);
                    break;
                }
            }
            Ok(Some(TtsWorkerEvent::Finished)) => {
                native.active=None;
                break;
            }
            Ok(Some(TtsWorkerEvent::Failed | TtsWorkerEvent::Cancelled)) => {
                // poll() has already removed the native Slot on a terminal event.
                native.active=None;
                failure=Some(PluginError::PluginRuntimeError);
                break;
            }
            Err(_) => {
                // The exact native lease state is unknown, not acknowledged.
                failure=Some(PluginError::WorkerUnreaped);
                break;
            }
            Ok(Some(TtsWorkerEvent::CleanupTimedOut)) => {
                failure=Some(PluginError::WorkerUnreaped);
                break;
            }
            Ok(Some(TtsWorkerEvent::TimedOut)) => {
                failure=Some(PluginError::WorkerTimeout);
                break;
            }
            Ok(None)=>tokio::time::sleep(Duration::from_millis(5)).await,
        }
    }
    if let Some(error)=failure {
        if native.active.is_some() && !native.stop_and_ack().await {
            return Err(PluginError::WorkerUnreaped);
        }
        return Err(error);
    }
    let rate=sample_rate.ok_or(PluginError::ApplicationError)?;
    let size=samples.checked_mul(2).and_then(|s|s.checked_add(44))
        .ok_or(PluginError::ApplicationError)?;
    let header=wav_header(rate,u32::try_from(samples*2).map_err(|_|PluginError::ApplicationError)?)
        .ok_or(PluginError::ApplicationError)?;
    file.rewind().await.map_err(|_|PluginError::StorageUnavailable)?;
    file.write_all(&header).await.map_err(|_|PluginError::StorageUnavailable)?;
    file.flush().await.map_err(|_|PluginError::StorageUnavailable)?;
    file.sync_all().await.map_err(|_|PluginError::StorageUnavailable)?;
    drop(file);
    drop(native);
    cleanup.retain=true;
    Ok(InvokeResult{
        result:serde_json::json!({
            "filename":"speech.wav","media_type":"audio/wav",
            "duration_ms":(samples*1000/rate as u64).max(1),
            "sample_rate_hz":rate,"size_bytes":size
        }),
        artifacts:vec![ArtifactData{
            source_path:path,filename:"speech.wav".into(),media_type:"audio/wav".into(),
            declared_size:Some(size),declared_sha256:None
        }],
        workspace:Some(workspace),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wav_header_is_mono_pcm16_with_actual_sample_rate() {
        let h=wav_header(48_000,960).unwrap();
        assert_eq!(&h[0..4],b"RIFF");
        assert_eq!(&h[8..12],b"WAVE");
        assert_eq!(u16::from_le_bytes([h[22],h[23]]),1);
        assert_eq!(u32::from_le_bytes([h[24],h[25],h[26],h[27]]),48_000);
        assert_eq!(u32::from_le_bytes([h[40],h[41],h[42],h[43]]),960);
    }
}
