use std::{env, path::PathBuf, process::ExitCode};

use serde::Deserialize;
use voice_agent_server::{
    audio::{
        DOWNLINK_FRAME_SAMPLES, DownlinkOpusEncoder, DownlinkPcmFrame, DownlinkResampler, Pcm16Mono,
    },
    providers::tts::zerotts_onnx::{ZeroTtsContract, ZeroTtsPcmStream},
};

#[derive(Deserialize)]
struct ParityFixture {
    text: String,
    max_frames: usize,
    token_ids: Vec<i64>,
    text_state_checksum: f64,
    checksum_tolerance: f64,
    first_frame_positions: Vec<i64>,
    first_seen_code_counts: Vec<usize>,
    first_frames: Vec<Vec<i32>>,
    eoa_frame_index: usize,
    eoa_frame: Vec<i32>,
}

fn required_path(name: &str) -> Result<PathBuf, String> {
    env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} must name a verified installed ZeroTTS artifact"))
}

fn write_wav(path: &std::path::Path, sample_rate: u32, samples: &[i16]) -> Result<(), String> {
    let data_bytes = u32::try_from(samples.len() * 2).map_err(|error| error.to_string())?;
    let mut wav = Vec::with_capacity(44 + data_bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, wav).map_err(|error| format!("{}: {error}", path.display()))
}

fn pcm16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn run() -> Result<(), String> {
    let fixture: ParityFixture = serde_json::from_str(include_str!(
        "../../tests/fixtures/zerotts_core_parity.json"
    ))
    .map_err(|error| format!("invalid checked-in ZeroTTS parity fixture: {error}"))?;
    let core = ZeroTtsContract::load_engine(
        &required_path("ZEROTTS_CONFIG")?,
        &required_path("ZEROTTS_TOKENIZER")?,
        &required_path("ZEROTTS_MAICHI_VOICE")?,
        &required_path("ZEROTTS_TEXT_ENCODER")?,
        &required_path("ZEROTTS_PREFIX_STEP")?,
        &required_path("ZEROTTS_LOCAL_FRAME_DECODE")?,
        &required_path("ZEROTTS_CODEC_DECODE_FULL")?,
        &required_path("ZEROTTS_CODEC_DECODE_STEP")?,
        &required_path("ZEROTTS_CODEC_SHARED_DATA")?,
        &required_path("ZEROTTS_CODEC_METADATA")?,
        &required_path("ZEROTTS_SILENCE_FRAME")?,
        &required_path("VOICE_ONNX_RUNTIME_LIB")?,
        1,
    )
    .map_err(|error| error.to_string())?;
    let result = core
        .synthesize_codes(&fixture.text, fixture.max_frames)
        .map_err(|error| error.to_string())?;
    let next_result = core
        .synthesize_codes(&fixture.text, fixture.max_frames)
        .map_err(|error| error.to_string())?;
    if result.token_ids != fixture.token_ids {
        return Err("token IDs differ from the checked-in parity fixture".into());
    }
    if (result.text_state_checksum - fixture.text_state_checksum).abs() > fixture.checksum_tolerance
    {
        return Err(format!(
            "text-state checksum differs from the checked-in parity fixture: actual={}, expected={}",
            result.text_state_checksum, fixture.text_state_checksum
        ));
    }
    let count = fixture.first_frames.len();
    if result.frames.get(..count) != Some(fixture.first_frames.as_slice())
        || result.frame_positions.get(..count) != Some(fixture.first_frame_positions.as_slice())
        || result.seen_code_counts.get(..count) != Some(fixture.first_seen_code_counts.as_slice())
    {
        return Err(
            "initial frame, position, or seen-mask checkpoint differs from the parity fixture"
                .into(),
        );
    }
    if result.eoa != Some(fixture.eoa_frame_index)
        || result.frames.get(fixture.eoa_frame_index) != Some(&fixture.eoa_frame)
        || result.frames.len() != fixture.eoa_frame_index + 2
    {
        return Err("EOA frame and one trailing audio frame were not retained".into());
    }
    if result.token_ids != next_result.token_ids
        || result.frames != next_result.frames
        || result.eoa != next_result.eoa
        || (result.text_state_checksum - next_result.text_state_checksum).abs()
            > fixture.checksum_tolerance
    {
        return Err("a second operation inherited stale ZeroTTS state".into());
    }
    let mut chunks = Vec::new();
    let mut stream = ZeroTtsPcmStream::new(&core).map_err(|error| error.to_string())?;
    stream
        .synthesize(&fixture.text, fixture.max_frames, &mut |pcm| {
            chunks.push(pcm);
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    if chunks.len() != 4 {
        return Err("ZeroTTS cold codec did not use 4, 8, 16, then final frames".into());
    }
    if chunks.iter().any(|pcm| {
        pcm.sample_rate_hz() != 48_000
            || pcm.samples().is_empty()
            || pcm.samples().iter().any(|sample| !sample.is_finite())
    }) {
        return Err("ZeroTTS codec did not produce finite 48 kHz PCM".into());
    }
    let mut next_chunks = Vec::new();
    stream
        .synthesize(&fixture.text, fixture.max_frames, &mut |pcm| {
            next_chunks.push(pcm);
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    if next_chunks.len() != 3
        || next_chunks.iter().any(|pcm| {
            pcm.sample_rate_hz() != 48_000
                || pcm.samples().is_empty()
                || pcm.samples().iter().any(|sample| !sample.is_finite())
        })
    {
        return Err("ZeroTTS continuous codec stream did not produce finite PCM".into());
    }
    let pcm = chunks
        .into_iter()
        .flat_map(|pcm| pcm.samples().to_vec())
        .collect::<Vec<_>>();
    let capture_pcm = if let Some(text) = env::var_os("ZEROTTS_DIAGNOSTIC_TEXT") {
        let mut diagnostic = ZeroTtsPcmStream::new(&core).map_err(|error| error.to_string())?;
        let mut samples = Vec::new();
        diagnostic
            .synthesize(&text.to_string_lossy(), 256, &mut |chunk| {
                samples.extend_from_slice(chunk.samples());
                Ok(())
            })
            .map_err(|error| error.to_string())?;
        samples
    } else {
        pcm.clone()
    };
    let raw_first = capture_pcm
        .get(..DOWNLINK_FRAME_SAMPLES * 2)
        .ok_or("ZeroTTS PCM is shorter than one canonical downlink frame")?;
    let mut faded_first = raw_first.to_vec();
    for (index, sample) in faded_first[..384].iter_mut().enumerate() {
        *sample *= index as f32 / 384.0;
    }
    let downlink = DownlinkResampler::new_48k_to_24k()
        .process(&faded_first)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(pcm16)
        .collect::<Vec<_>>();
    let mut frame = downlink
        .get(..DOWNLINK_FRAME_SAMPLES)
        .ok_or("ZeroTTS PCM is shorter than one canonical downlink frame")?
        .to_vec();
    frame.resize(DOWNLINK_FRAME_SAMPLES, 0);
    let packet = DownlinkOpusEncoder::new(4_000)
        .map_err(|error| error.to_string())?
        .encode(
            DownlinkPcmFrame::try_new(Pcm16Mono::new(frame.clone()))
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    let mut decoder =
        opus2::Decoder::new(24_000, opus2::Channels::Mono).map_err(|error| error.to_string())?;
    let mut decoded = [0_i16; DOWNLINK_FRAME_SAMPLES];
    if decoder
        .decode(packet.as_bytes(), &mut decoded, false)
        .map_err(|error| error.to_string())?
        != DOWNLINK_FRAME_SAMPLES
    {
        return Err("canonical Opus packet did not decode to one 60 ms frame".into());
    }
    if let Some(directory) = env::var_os("ZEROTTS_FIRST_PACKET_CAPTURE_DIR") {
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        write_wav(
            &directory.join("A-provider-48k.wav"),
            48_000,
            &raw_first.iter().copied().map(pcm16).collect::<Vec<_>>(),
        )?;
        write_wav(&directory.join("B-resampled-24k.wav"), 24_000, &frame)?;
        write_wav(&directory.join("C-opus-decoded-24k.wav"), 24_000, &decoded)?;
    }
    if let Some(path) = env::var_os("ZEROTTS_DOWNLINK_OPUS_PATH") {
        std::fs::write(&path, packet.as_bytes()).map_err(|error| {
            format!(
                "write canonical downlink Opus packet to {}: {error}",
                PathBuf::from(path).display()
            )
        })?;
    }
    println!(
        "ZeroTTS parity, codec, and canonical Opus accepted: {} frames, EOA frame {}, {} PCM samples",
        result.frames.len(),
        fixture.eoa_frame_index,
        pcm.len(),
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ZeroTTS core check failed: {error}");
            ExitCode::FAILURE
        }
    }
}
