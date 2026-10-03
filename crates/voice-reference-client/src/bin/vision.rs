use clap::Parser;
use std::{path::PathBuf, time::Duration};
#[derive(Debug, Parser)]
#[command(
    name = "voice-vision-client",
    about = "POST a camera image to the Xiaozhi-compatible Vision API"
)]
struct Args {
    #[arg(long)]
    url: String,
    #[arg(long, default_value = "")]
    token: String,
    #[arg(long, default_value = "reference-client-01")]
    device_id: String,
    #[arg(long, default_value = "reference-client")]
    client_id: String,
    #[arg(long)]
    image: PathBuf,
    #[arg(long, default_value = "Mô tả hình ảnh này")]
    question: String,
    #[arg(long, default_value_t = 60)]
    timeout_secs: u64,
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let report =
        voice_reference_client::run_vision_request(voice_reference_client::VisionRequestOptions {
            vision_url: args.url,
            token: args.token,
            device_id: args.device_id,
            client_id: args.client_id,
            question: args.question,
            image_path: args.image,
            timeout: Duration::from_secs(args.timeout_secs),
        })
        .await?;
    println!(
        "Vision API: PASS\nHTTP: {}\nImage bytes: {}\nResponse: {}",
        report.http_status, report.image_bytes, report.response_text
    );
    Ok(())
}
