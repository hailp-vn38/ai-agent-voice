use std::sync::atomic::{AtomicBool, Ordering};

use crate::{audio::PcmF32Mono, providers::tts::TtsError};

use super::{KOKORO_SAMPLE_RATE_HZ, PCM_CHUNK_SAMPLES, emit_pcm_chunks};

#[test]
fn complete_waveform_is_emitted_as_bounded_24khz_chunks() {
    let samples = (0..PCM_CHUNK_SAMPLES * 2 + 17)
        .map(|index| index as f32 / 10_000.0)
        .collect::<Vec<_>>();
    let cancelled = AtomicBool::new(false);
    let mut chunks = Vec::<PcmF32Mono>::new();

    emit_pcm_chunks(&samples, &cancelled, &mut |pcm| {
        chunks.push(pcm);
        Ok(())
    })
    .expect("emit valid waveform");

    assert_eq!(chunks.len(), 3);
    assert!(
        chunks
            .iter()
            .all(|chunk| chunk.sample_rate_hz() == KOKORO_SAMPLE_RATE_HZ)
    );
    assert!(
        chunks[..2]
            .iter()
            .all(|chunk| chunk.samples().len() == PCM_CHUNK_SAMPLES)
    );
    assert_eq!(chunks[2].samples().len(), 17);
    assert_eq!(
        chunks
            .iter()
            .flat_map(|chunk| chunk.samples().iter().copied())
            .collect::<Vec<_>>(),
        samples
    );
}

#[test]
fn cancellation_stops_emission_between_pcm_chunks() {
    let samples = vec![0.25; PCM_CHUNK_SAMPLES * 2];
    let cancelled = AtomicBool::new(false);
    let mut delivered = 0;

    let result = emit_pcm_chunks(&samples, &cancelled, &mut |_| {
        delivered += 1;
        cancelled.store(true, Ordering::Release);
        Ok(())
    });

    assert!(matches!(result, Err(TtsError::Failed)));
    assert_eq!(delivered, 1);
}

#[test]
fn invalid_waveform_is_rejected_before_delivery() {
    let cancelled = AtomicBool::new(false);
    let mut delivered = false;

    for samples in [&[][..], &[f32::NAN][..]] {
        let result = emit_pcm_chunks(samples, &cancelled, &mut |_| {
            delivered = true;
            Ok(())
        });
        assert!(matches!(result, Err(TtsError::Failed)));
    }

    assert!(!delivered);
}
