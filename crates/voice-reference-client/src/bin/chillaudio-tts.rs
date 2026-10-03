use anyhow::Context;
use clap::Parser;
use std::{path::PathBuf, time::Duration};
use voice_reference_client::chillaudio::{CHILLAUDIO_VOICES, ChillAudioClient, ChillAudioConfig};

#[derive(Debug, Parser)]
#[command(
    name = "chillaudio-tts",
    about = "Direct ChillAudio WebSocket TTS reference runner"
)]
struct Args {
    /// Text to synthesize.
    text: String,
    #[arg(long, default_value = "BV421_vivn_streaming")]
    voice: String,
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value_t = 12_000)]
    timeout_ms: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    anyhow::ensure!(
        CHILLAUDIO_VOICES.contains(&args.voice.as_str()),
        "unsupported ChillAudio voice: {}",
        args.voice
    );

    let config = ChillAudioConfig {
        voice: args.voice,
        timeout: Duration::from_millis(args.timeout_ms),
        ..Default::default()
    };

    if let Some(parent) = args
        .out
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "ChillAudio output directory creation failed: {}",
                parent.display()
            )
        })?;
    }
    let result = ChillAudioClient::new(config).synthesize(&args.text).await?;
    std::fs::write(&args.out, &result.audio).with_context(|| {
        format!(
            "ChillAudio output file write failed: {}",
            args.out.display()
        )
    })?;

    println!(
        "first_audio_ms={:.1} total_ms={:.1} chunks={} bytes={} out={}",
        result.first_audio_ms.unwrap_or(f64::NAN),
        result.total_ms,
        result.binary_chunks,
        result.audio.len(),
        args.out.display(),
    );
    Ok(())
}
