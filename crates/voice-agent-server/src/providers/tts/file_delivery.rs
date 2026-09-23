use std::{
    fs::{self, File, OpenOptions},
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use uuid::Uuid;

use super::{PcmF32Mono, TtsError};

const FILE_READ_SAMPLES: usize = 2_880;

struct TemporaryWav {
    path: PathBuf,
}

impl Drop for TemporaryWav {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn file_error(error: impl std::fmt::Display) -> TtsError {
    TtsError::TemporaryAudio(error.to_string())
}

pub(super) fn deliver_via_temporary_wav(
    pcm: PcmF32Mono,
    cancelled: &AtomicBool,
    on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
) -> Result<(), TtsError> {
    let path = std::env::temp_dir().join(format!("voice-agent-zerotts-{}.wav", Uuid::new_v4()));
    deliver_at_path(path, pcm, cancelled, on_pcm)
}

fn deliver_at_path(
    path: PathBuf,
    pcm: PcmF32Mono,
    cancelled: &AtomicBool,
    on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
) -> Result<(), TtsError> {
    if cancelled.load(Ordering::Acquire) {
        return Err(TtsError::Failed);
    }
    if pcm.sample_rate_hz() != 48_000
        || pcm.samples().is_empty()
        || pcm.samples().iter().any(|sample| !sample.is_finite())
    {
        return Err(TtsError::TemporaryAudio(
            "PCM must be finite, non-empty, mono 48 kHz".into(),
        ));
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file: File = options.open(&path).map_err(file_error)?;
    let temporary = TemporaryWav { path };
    let spec = WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let sample_count = pcm.samples().len();
    let mut writer = WavWriter::new(file, spec).map_err(file_error)?;
    for (index, sample) in pcm.samples().iter().copied().enumerate() {
        if index.is_multiple_of(4_096) && cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        writer.write_sample(sample).map_err(file_error)?;
    }
    writer.finalize().map_err(file_error)?;
    drop(pcm);
    tracing::info!(samples = sample_count, "ZeroTTS temporary WAV ready");
    if cancelled.load(Ordering::Acquire) {
        return Err(TtsError::Failed);
    }
    let mut reader = WavReader::open(&temporary.path).map_err(file_error)?;
    if reader.spec() != spec {
        return Err(TtsError::TemporaryAudio(
            "temporary WAV profile changed".into(),
        ));
    }
    let mut chunk = Vec::with_capacity(FILE_READ_SAMPLES);
    for sample in reader.samples::<f32>() {
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        chunk.push(sample.map_err(file_error)?);
        if chunk.len() == FILE_READ_SAMPLES {
            on_pcm(PcmF32Mono::new(
                std::mem::take(&mut chunk),
                spec.sample_rate,
            ))?;
        }
    }
    if !chunk.is_empty() {
        on_pcm(PcmF32Mono::new(chunk, spec.sample_rate))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replays_only_a_finalized_file_and_removes_it() {
        let path =
            std::env::temp_dir().join(format!("voice-agent-tts-test-{}.wav", Uuid::new_v4()));
        let samples = (0..6_100)
            .map(|index| index as f32 / 6_100.0)
            .collect::<Vec<_>>();
        let mut received = Vec::new();
        deliver_at_path(
            path.clone(),
            PcmF32Mono::new(samples.clone(), 48_000),
            &AtomicBool::new(false),
            &mut |pcm| {
                assert!(WavReader::open(&path).is_ok());
                received.extend_from_slice(pcm.samples());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(received, samples);
        assert!(!path.exists());
    }

    #[test]
    fn removes_file_when_delivery_is_cancelled() {
        let path =
            std::env::temp_dir().join(format!("voice-agent-tts-test-{}.wav", Uuid::new_v4()));
        let cancelled = AtomicBool::new(false);
        let result = deliver_at_path(
            path.clone(),
            PcmF32Mono::new(vec![0.25; 6_100], 48_000),
            &cancelled,
            &mut |_| {
                cancelled.store(true, Ordering::Release);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(!path.exists());
    }
}
