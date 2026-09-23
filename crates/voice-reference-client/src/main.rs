use clap::Parser;
use std::{path::PathBuf, time::Duration};

#[derive(Debug, Parser)]
#[command(
    name = "voice-reference-client",
    about = "Voice Protocol V1 client for a server text turn"
)]
struct Args {
    #[arg(long)]
    ota: String,
    #[arg(long, default_value = "reference-client-01")]
    device_id: String,
    #[arg(long, default_value = "reference-client")]
    client_id: String,
    text: String,
    #[arg(long, default_value_t = 60)]
    tts_start_timeout_secs: u64,
    #[arg(long, default_value_t = 120)]
    turn_timeout_secs: u64,
    #[arg(long, default_value_t = 250)]
    post_stop_quiet_period_ms: u64,
    /// Write decoded 24 kHz mono TTS audio after a successful server turn.
    #[arg(long)]
    debug_audio_file: Option<PathBuf>,
    /// Print privacy-safe lifecycle step names and WebSocket frame categories to stderr.
    #[arg(long)]
    debug_steps: bool,
    /// Serve deterministic Device MCP tools for this text turn.
    #[arg(long)]
    mcp: bool,
    #[arg(long, default_value_t = 10)]
    mcp_initial_value: i32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if args.mcp {
        let report =
            voice_reference_client::run_mcp_text_turn(voice_reference_client::McpTextTurnOptions {
                ota_url: args.ota,
                device_id: args.device_id,
                client_id: args.client_id,
                text: args.text,
                initial_value: args.mcp_initial_value,
                turn_timeout: Duration::from_secs(args.turn_timeout_secs),
            })
            .await?;
        println!("MCP discovery: PASS");
        println!("tools: {}", report.discovered_tools.join(", "));
        println!("tool calls: {}", report.received_calls.len());
        println!("final value: {}", report.final_value);
        println!(
            "TTS lifecycle: {}",
            if report.tts_started && report.tts_finished {
                "complete"
            } else {
                "incomplete"
            }
        );
        return Ok(());
    }
    let report = voice_reference_client::run_text_turn(voice_reference_client::TextTurnRequest {
        ota_url: args.ota,
        device_id: args.device_id,
        client_id: args.client_id,
        text: args.text,
        config: voice_reference_client::TextTurnConfig {
            tts_start_timeout: Duration::from_secs(args.tts_start_timeout_secs),
            turn_timeout: Duration::from_secs(args.turn_timeout_secs),
            post_stop_quiet_period: Duration::from_millis(args.post_stop_quiet_period_ms),
            debug_audio_file: args.debug_audio_file,
            debug_steps: args.debug_steps,
        },
    })
    .await?;

    println!(
        "validated server text turn with {} canonical downlink packets",
        report.binary_packets
    );
    if let Some(path) = report.debug_audio_file {
        println!("wrote decoded TTS WAV to {}", path.display());
    }
    Ok(())
}
