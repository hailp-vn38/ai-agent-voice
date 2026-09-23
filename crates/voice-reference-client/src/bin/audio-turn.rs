use clap::{Parser, ValueEnum};
use std::{path::PathBuf, time::Duration};
use voice_reference_client::{AudioListenMode, AudioTurnRequest};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Mode {
    Manual,
    Auto,
}

#[derive(Debug, Parser)]
#[command(
    name = "audio-turn",
    about = "Replay a WAV through Voice Protocol V1 and verify STT"
)]
struct Args {
    #[arg(long)]
    ota: String,
    #[arg(long, default_value = "reference-audio-01")]
    device_id: String,
    #[arg(long, default_value = "reference-audio")]
    client_id: String,
    #[arg(long, default_value = "docs/audio.wav")]
    wav: PathBuf,
    #[arg(long, value_enum, default_value_t = Mode::Auto)]
    mode: Mode,
    #[arg(long)]
    expected_stt_suffix: String,
    /// Number of 60 ms silence frames sent after the WAV and before Manual stop.
    #[arg(long)]
    trailing_silence_frames: Option<usize>,
    #[arg(long, default_value_t = 30)]
    timeout_secs: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let report = voice_reference_client::run_audio_turn(AudioTurnRequest {
        ota_url: args.ota,
        device_id: args.device_id,
        client_id: args.client_id,
        wav_file: args.wav,
        mode: match args.mode {
            Mode::Manual => AudioListenMode::Manual,
            Mode::Auto => AudioListenMode::Auto,
        },
        expected_stt_suffix: args.expected_stt_suffix,
        trailing_silence_frames: args.trailing_silence_frames.unwrap_or(match args.mode {
            Mode::Manual => 0,
            Mode::Auto => 20,
        }),
        timeout: Duration::from_secs(args.timeout_secs),
    })
    .await?;
    println!(
        "STT suffix: PASS ({} canonical uplink packets)",
        report.uplink_packets
    );
    Ok(())
}
