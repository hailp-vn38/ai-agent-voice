use super::{EnrollmentRuntime, PromptAssets};
use crate::config::{EnrollmentConfig, EnrollmentTransport};

fn fixtures() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("enrollment-audio-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&path).unwrap();
    for name in std::iter::once("intro".to_string()).chain((0..10).map(|n| n.to_string())) {
        let mut writer = hound::WavWriter::create(path.join(format!("{name}.wav")), hound::WavSpec {
            channels: 1, sample_rate: 24_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int,
        }).unwrap();
        for _ in 0..2_400 { writer.write_sample(1_000_i16).unwrap(); }
        writer.finalize().unwrap();
    }
    path
}

#[test]
fn canonical_packets_decode_and_invalid_code_or_assets_fail_closed() {
    let path = fixtures();
    let assets = PromptAssets::load(&path).unwrap();
    let packets = assets.encode("000001").unwrap();
    assert!(!packets.is_empty() && packets.len() <= 250);
    let mut decoder = opus2::Decoder::new(24_000, opus2::Channels::Mono).unwrap();
    for packet in packets {
        let mut pcm = [0_i16; 2_880];
        assert_eq!(decoder.decode(&packet, &mut pcm, false).unwrap(), 1_440);
    }
    assert!(assets.encode("12345").is_err());
    assert!(assets.encode("１２３４５６").is_err());
    assert!(assets.encode("12345a").is_err());
    std::fs::remove_file(path.join("0.wav")).unwrap();
    assert!(PromptAssets::load(&path).is_err());
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn static_runtime_has_independent_bounded_connection_capacity() {
    let path = fixtures();
    let config = EnrollmentConfig { enabled: true, ws_max_connections: 1, prompt_assets_dir: path.clone(), ..Default::default() };
    let runtime = EnrollmentRuntime::prepare(&config).await.unwrap().unwrap();
    let permit = runtime.try_connection().unwrap();
    assert!(runtime.try_connection().is_none());
    assert!(!runtime.encode("042731".into()).await.unwrap().is_empty());
    drop(permit);
    assert!(runtime.try_connection().is_some());
    let ota = EnrollmentConfig { transport: EnrollmentTransport::Ota, ..config };
    assert!(EnrollmentRuntime::prepare(&ota).await.unwrap().is_none());
    std::fs::remove_dir_all(path).unwrap();
}
