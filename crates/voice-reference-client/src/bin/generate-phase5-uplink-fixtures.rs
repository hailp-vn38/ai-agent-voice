use std::{f32::consts::TAU, fs, path::Path};

use opus2::{Application, Bitrate, Channels, Encoder};

const OUTPUT: &str = "crates/voice-reference-client/tests/fixtures";

fn main() -> anyhow::Result<()> {
    fs::create_dir_all(OUTPUT)?;
    for (name, samples) in [
        ("01-silence", vec![0; 960]),
        ("02-speech-a", tone(440.0, 9_000)),
        ("03-silence", vec![0; 960]),
        ("04-speech-b", tone(660.0, 7_000)),
        ("05-silence", vec![0; 960]),
    ] {
        fs::write(
            Path::new(OUTPUT).join(format!("phase5-uplink-{name}.opus")),
            encode(&samples)?,
        )?;
    }
    Ok(())
}

fn tone(hz: f32, amplitude: i16) -> Vec<i16> {
    (0..960)
        .map(|sample| (amplitude as f32 * (TAU * hz * sample as f32 / 16_000.0).sin()) as i16)
        .collect()
}

fn encode(samples: &[i16]) -> anyhow::Result<Vec<u8>> {
    let mut encoder = Encoder::new(16_000, Channels::Mono, Application::Voip)?;
    encoder.set_bitrate(Bitrate::Bits(32_000))?;
    encoder.set_vbr(true)?;
    encoder.set_vbr_constraint(true)?;
    encoder.set_dtx(false)?;
    encoder.set_inband_fec(false)?;
    encoder.set_packet_loss_perc(0)?;
    encoder.set_complexity(10)?;
    let mut output = [0_u8; 4_000];
    let length = encoder.encode(samples, &mut output)?;
    Ok(output[..length].to_vec())
}
