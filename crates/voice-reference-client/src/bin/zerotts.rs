//! Local ZeroTTS reference runner. It writes provider PCM before server resampling or Opus.
//! The inference loop is kept here so experiments do not change the server's delivery path.

#[path = "zerotts/codec.rs"]
mod codec;
#[path = "zerotts/contract.rs"]
mod contract;
#[path = "zerotts/synthesis.rs"]
mod synthesis;
#[path = "zerotts/text.rs"]
mod text;

use codec::*;
use contract::*;
use synthesis::*;
use text::*;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use ndarray::{Array0, Array3};
use ndarray_npy::NpzReader;
use ort::{
    session::Session,
    value::{Tensor, TensorElementType, ValueType},
};
use serde::Deserialize;
use tokenizers::Tokenizer;
use unicode_normalization::UnicodeNormalization;

use clap::Parser;
use std::sync::OnceLock;
use std::time::Instant;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TtsError {
    #[error("ZeroTTS contract is incompatible: {0}")]
    IncompatibleContract(String),
}

pub struct PcmF32Mono {
    samples: Vec<f32>,
    sample_rate_hz: u32,
}

impl PcmF32Mono {
    fn new(samples: Vec<f32>, sample_rate_hz: u32) -> Self {
        Self {
            samples,
            sample_rate_hz,
        }
    }

    fn samples(&self) -> &[f32] {
        &self.samples
    }

    fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }
}

fn initialize_ort(path: &Path) -> Result<(), TtsError> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    INIT.get_or_init(|| {
        if !path.is_file() {
            return Err(format!(
                "ONNX Runtime library is missing: {}",
                path.display()
            ));
        }
        ort::init_from(path)
            .map_err(|error| error.to_string())?
            .commit()
            .then_some(())
            .ok_or_else(|| "ONNX Runtime was initialized before this runner".to_owned())
    })
    .as_ref()
    .map_err(|error| TtsError::IncompatibleContract(error.clone()))
    .copied()
}

fn contract_error(error: impl std::fmt::Display) -> TtsError {
    TtsError::IncompatibleContract(error.to_string())
}

#[derive(Parser)]
#[command(
    name = "zerotts",
    about = "Local 48 kHz ZeroTTS streaming reference runner"
)]
struct Args {
    /// Written text; Vietnamese spoken-form normalization is enabled by default.
    text: String,
    #[arg(long, default_value = "models/zerotts")]
    model_dir: PathBuf,
    #[arg(long)]
    ort_library: Option<PathBuf>,
    #[arg(long, default_value_t = 2)]
    threads: i32,
    #[arg(long, default_value_t = 1500)]
    max_frames: usize,
    #[arg(long, default_value_t = 2)]
    repeats: usize,
    #[arg(long)]
    no_warmup: bool,
    /// Pass written text directly to the tokenizer, matching Python synthesize_stream.
    #[arg(long)]
    no_text_norm: bool,
    /// Write the final run as mono float32 PCM WAV at 48 kHz.
    #[arg(long)]
    out: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    anyhow::ensure!(!args.text.trim().is_empty(), "text must not be empty");
    anyhow::ensure!(
        args.repeats > 0 && args.max_frames > 0,
        "repeats and max-frames must be positive"
    );
    let library = args
        .ort_library
        .or_else(|| std::env::var_os("VOICE_ONNX_RUNTIME_LIB").map(PathBuf::from))
        .ok_or_else(|| anyhow::anyhow!("pass --ort-library or set VOICE_ONNX_RUNTIME_LIB"))?;
    let text = if args.no_text_norm {
        args.text.clone()
    } else {
        normalize_vi_text(&args.text)
    };
    let started = Instant::now();
    let contract = ZeroTtsContract::load_engine(&args.model_dir, &library, args.threads)?;
    let mut stream = ZeroTtsPcmStream::new(&contract)?;
    if !args.no_warmup {
        stream.synthesize("ZeroTTS startup readiness.", 256, &mut |_| Ok(()))?;
        stream.reset()?;
    }
    eprintln!("init_ms={:.1}", started.elapsed().as_secs_f64() * 1000.0);

    let mut final_pcm = Vec::new();
    for run in 0..args.repeats {
        let started = Instant::now();
        let mut first_ms = None;
        let mut chunks = 0;
        let mut pcm = Vec::new();
        stream.synthesize(&text, args.max_frames, &mut |chunk| {
            if chunk.sample_rate_hz() != 48_000 {
                return Err(TtsError::IncompatibleContract(
                    "codec did not return 48 kHz PCM".into(),
                ));
            }
            if first_ms.is_none() {
                first_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
            }
            chunks += 1;
            pcm.extend_from_slice(chunk.samples());
            Ok(())
        })?;
        let elapsed = started.elapsed().as_secs_f64();
        let duration = pcm.len() as f64 / 48_000.0;
        println!(
            "run={} first_pcm_ms={:.1} synthesis_ms={:.1} audio_ms={:.1} rtf={:.3} chunks={} samples={}",
            run + 1,
            first_ms.unwrap_or(f64::NAN),
            elapsed * 1000.0,
            duration * 1000.0,
            elapsed / duration,
            chunks,
            pcm.len()
        );
        final_pcm = pcm;
        stream.reset()?;
    }
    let mut writer = hound::WavWriter::create(
        &args.out,
        hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for sample in final_pcm {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    println!("wav={}", args.out.display());
    Ok(())
}
