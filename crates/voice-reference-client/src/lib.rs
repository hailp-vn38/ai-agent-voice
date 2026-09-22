//! Reusable wire checks owned by the independent Voice Protocol Client.

use anyhow::{Context, bail};
use opus2::{Channels, Decoder};

/// Decodes exactly one canonical downlink Opus packet.
pub fn decode_canonical_downlink_opus_packet(packet: &[u8]) -> anyhow::Result<usize> {
    if packet.is_empty() {
        bail!("canonical downlink packet is empty");
    }
    let mut decoder =
        Decoder::new(24_000, Channels::Mono).context("create canonical downlink Opus decoder")?;
    let mut pcm = [0_i16; 1_440];
    let decoded = decoder
        .decode(packet, &mut pcm, false)
        .context("decode canonical downlink Opus packet")?;
    if decoded != pcm.len() {
        bail!(
            "downlink packet decoded to {decoded} samples; expected one 60 ms 24 kHz frame ({})",
            pcm.len()
        );
    }
    Ok(decoded)
}
