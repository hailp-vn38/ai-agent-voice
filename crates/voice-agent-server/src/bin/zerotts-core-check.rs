use std::{env, path::PathBuf, process::ExitCode};

use serde::Deserialize;
use voice_agent_server::providers::tts::zerotts_onnx::ZeroTtsContract;

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
        return Err("text-state checksum differs from the checked-in parity fixture".into());
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
    {
        return Err("EOA frame was not retained at the checked-in parity checkpoint".into());
    }
    if result.token_ids != next_result.token_ids
        || result.frames != next_result.frames
        || result.eoa != next_result.eoa
        || (result.text_state_checksum - next_result.text_state_checksum).abs()
            > fixture.checksum_tolerance
    {
        return Err("a second operation inherited stale ZeroTTS state".into());
    }
    println!(
        "ZeroTTS parity accepted: {} frames, EOA frame {}",
        result.frames.len(),
        fixture.eoa_frame_index
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
