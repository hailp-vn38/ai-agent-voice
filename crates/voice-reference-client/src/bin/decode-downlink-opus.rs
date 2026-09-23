use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
struct Args {
    packet: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let packet = std::fs::read(&args.packet)?;
    let samples = voice_reference_client::decode_canonical_downlink_opus_packet(&packet)?;
    println!("decoded {samples} canonical downlink samples");
    Ok(())
}
