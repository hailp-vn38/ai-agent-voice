//! The ZeroTTS startup warmup contract: a provider that reports readiness must have produced real
//! audio at the rate the delivery pipeline expects.

use voice_agent_server::providers::tts::{WarmupPcm, validate_warmup_pcm};

#[test]
fn warmup_pcm_requires_terminal_finite_non_empty_48khz_mono_audio() {
    for pcm in [
        WarmupPcm::new(24_000, 1, vec![0.0]),
        WarmupPcm::new(48_000, 2, vec![0.0]),
        WarmupPcm::new(48_000, 1, Vec::new()),
        WarmupPcm::new(48_000, 1, vec![f32::NAN]),
    ] {
        assert!(validate_warmup_pcm(pcm).is_err());
    }

    validate_warmup_pcm(WarmupPcm::new(48_000, 1, vec![0.0, 0.25, -0.25])).unwrap();
}
