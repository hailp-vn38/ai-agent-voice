use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use voice_agent_server::{
    benchmark::{
        AsrBenchmarkResult, AsrFeedMode, LlmBenchmarkResult, MetricSummary, VadBenchmarkResult,
        run_asr_provider, run_llm_provider, run_vad_provider, summarize,
    },
    config::{AppConfig, BenchmarkTarget},
    models::prepare,
    providers::{
        compiled_provider_registry,
        llm::{ChatMessage, LlmRequest},
    },
};

#[derive(Deserialize)]
struct Workload {
    schema_version: u8,
    workload_version: String,
    items: Vec<WorkloadItem>,
}
#[derive(Deserialize)]
struct WorkloadItem {
    id: String,
    path: PathBuf,
}
struct Args {
    target: String,
    config: PathBuf,
    workload: PathBuf,
    feed: AsrFeedMode,
    warmup: usize,
    iterations: usize,
    output: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args()?;
    let registry = compiled_provider_registry();
    match args.target.as_str() {
        "asr" => {
            let workload = load_workload(&args.workload)?;
            let audio = load_audio(&args.workload, &workload)?;
            run_asr(args, workload, audio, registry)
        }
        "vad" => {
            let workload = load_workload(&args.workload)?;
            let audio = load_audio(&args.workload, &workload)?;
            run_vad(args, workload, audio, registry)
        }
        "llm" => run_llm(args, registry).await,
        _ => anyhow::bail!("target must be asr, vad, or llm"),
    }
}

#[derive(Deserialize)]
struct LlmWorkload {
    schema_version: u8,
    workload_version: String,
    items: Vec<LlmWorkloadItem>,
}
#[derive(Deserialize)]
struct LlmWorkloadItem {
    id: String,
    messages: Vec<LlmWorkloadMessage>,
}
#[derive(Deserialize)]
struct LlmWorkloadMessage {
    role: String,
    content: String,
}

async fn run_llm(
    args: Args,
    registry: &voice_agent_server::providers::ProviderRegistry,
) -> anyhow::Result<()> {
    let workload: LlmWorkload = serde_json::from_slice(&fs::read(&args.workload)?)?;
    if workload.schema_version != 1
        || workload.workload_version.trim().is_empty()
        || workload.items.is_empty()
    {
        anyhow::bail!("LLM workload must have schema_version=1, a version, and at least one item")
    }
    if workload
        .items
        .iter()
        .any(|item| item.id.trim().is_empty() || item.messages.is_empty())
    {
        anyhow::bail!("LLM workload item id and messages must not be empty")
    }
    let config = AppConfig::load_for_benchmark(&args.config, BenchmarkTarget::LlmProvider)?;
    let instance = &config.providers.llm.instances[&config.effective_agent().providers.llm];
    let factory = registry.llm_factory(instance.adapter())?;
    let provider = factory.build(instance)?;
    for item in &workload.items {
        for _ in 0..args.warmup {
            let sample = run_llm_provider(provider.as_ref(), request_from_item(item)?).await?;
            qualify_llm(&sample)?;
        }
    }
    let mut samples = Vec::new();
    for _ in 0..args.iterations {
        for item in &workload.items {
            let sample = run_llm_provider(provider.as_ref(), request_from_item(item)?).await?;
            qualify_llm(&sample)?;
            samples.push(sample);
        }
    }
    let result = LlmBenchmarkResult {
        schema_version: 1,
        status: "passed",
        workload_version: workload.workload_version,
        warmup_runs: args.warmup,
        iterations: args.iterations,
        ttft_ms: optional_summary(samples.iter().filter_map(|sample| sample.ttft_ms)),
        total_ms: summary(samples.iter().map(|sample| sample.total_ms)),
        text_delta_count: summary(samples.iter().map(|sample| sample.text_delta_count as f64)),
        output_chars: summary(samples.iter().map(|sample| sample.output_chars as f64)),
        tool_call_count: summary(samples.iter().map(|sample| sample.tool_call_count as f64)),
        samples,
    };
    publish(&result, args.output.as_deref())
}

fn request_from_item(item: &LlmWorkloadItem) -> anyhow::Result<LlmRequest> {
    let messages = item
        .messages
        .iter()
        .map(|message| match message.role.as_str() {
            "system" => Ok(ChatMessage::System {
                content: message.content.clone(),
            }),
            "user" => Ok(ChatMessage::User {
                content: message.content.clone(),
            }),
            "assistant" => Ok(ChatMessage::AssistantText {
                content: message.content.clone(),
            }),
            _ => anyhow::bail!("LLM workload role must be system, user, or assistant"),
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    if messages.iter().any(|message| matches!(message, ChatMessage::System { content } | ChatMessage::User { content } | ChatMessage::AssistantText { content } if content.trim().is_empty())) {
        anyhow::bail!("LLM workload message content must not be empty")
    }
    Ok(LlmRequest {
        messages,
        tools: Vec::new(),
    })
}

fn qualify_llm(sample: &voice_agent_server::benchmark::LlmRunMetrics) -> anyhow::Result<()> {
    if sample.ttft_ms.is_none() {
        anyhow::bail!("LLM qualification failed: no non-empty text delta")
    }
    Ok(())
}

fn run_asr(
    args: Args,
    workload: Workload,
    audio: Vec<(String, Vec<f32>)>,
    registry: &voice_agent_server::providers::ProviderRegistry,
) -> anyhow::Result<()> {
    let config = AppConfig::load_for_benchmark(&args.config, BenchmarkTarget::AsrProvider)?;
    let instance = &config.providers.asr.instances[&config.effective_agent().providers.asr];
    let factory = registry.asr_factory(instance.adapter())?;
    let identity = factory.model_identity(instance)?;
    let model = prepare(
        &config.deployment.model_manifest,
        &config.deployment.models.root,
        config.deployment.models.offline,
        identity,
        factory.adapter(),
        &config.deployment,
    )?;
    let provider = factory.build(instance, &model)?;
    for (_, samples) in &audio {
        for _ in 0..args.warmup {
            run_asr_provider(provider.as_ref(), samples, args.feed)?;
        }
    }
    let mut samples = Vec::new();
    for _ in 0..args.iterations {
        for (_, pcm) in &audio {
            samples.push(run_asr_provider(provider.as_ref(), pcm, args.feed)?);
        }
    }
    if samples.iter().any(|sample| sample.final_text_chars == 0) {
        anyhow::bail!("ASR qualification failed: a final text was empty")
    }
    let result = AsrBenchmarkResult {
        schema_version: 1,
        status: "passed",
        workload_version: workload.workload_version,
        feed: args.feed,
        warmup_runs: args.warmup,
        iterations: args.iterations,
        session_open_ms: summary(samples.iter().map(|s| s.session_open_ms)),
        first_partial_compute_ms: optional_summary(
            samples.iter().filter_map(|s| s.first_partial_compute_ms),
        ),
        first_partial_wall_ms: optional_summary(
            samples.iter().filter_map(|s| s.first_partial_wall_ms),
        ),
        finish_ms: summary(samples.iter().map(|s| s.finish_ms)),
        compute_total_ms: summary(samples.iter().map(|s| s.compute_total_ms)),
        wall_total_ms: summary(samples.iter().map(|s| s.wall_total_ms)),
        compute_rtf: summary(
            samples
                .iter()
                .map(|s| s.compute_total_ms / s.audio_duration_ms),
        ),
        samples,
    };
    publish(&result, args.output.as_deref())
}

fn run_vad(
    args: Args,
    workload: Workload,
    audio: Vec<(String, Vec<f32>)>,
    registry: &voice_agent_server::providers::ProviderRegistry,
) -> anyhow::Result<()> {
    let config = AppConfig::load_for_benchmark(&args.config, BenchmarkTarget::VadProvider)?;
    let instance = &config.providers.vad.instances[&config.effective_agent().providers.vad];
    let factory = registry.vad_factory(instance.adapter())?;
    let identity = factory.model_identity(instance)?;
    let model = prepare(
        &config.deployment.model_manifest,
        &config.deployment.models.root,
        config.deployment.models.offline,
        identity,
        factory.adapter(),
        &config.deployment,
    )?;
    let provider = factory.build(instance, &config.runtime, &model)?;
    for (_, samples) in &audio {
        for _ in 0..args.warmup {
            run_vad_provider(provider.as_ref(), samples)?;
        }
    }
    let mut samples = Vec::new();
    for _ in 0..args.iterations {
        for (_, pcm) in &audio {
            samples.push(run_vad_provider(provider.as_ref(), pcm)?);
        }
    }
    let result = VadBenchmarkResult {
        schema_version: 1,
        status: "passed",
        workload_version: workload.workload_version,
        warmup_runs: args.warmup,
        iterations: args.iterations,
        session_open_ms: summary(samples.iter().map(|s| s.session_open_ms)),
        frame_latency_us: summary(
            samples
                .iter()
                .flat_map(|s| s.frame_latencies_us.iter().copied()),
        ),
        frames_per_second: summary(samples.iter().map(|s| s.frames_per_second)),
        compute_rtf: summary(samples.iter().map(|s| s.compute_rtf)),
        samples,
    };
    publish(&result, args.output.as_deref())
}

fn parse_args() -> anyhow::Result<Args> {
    let mut values = env::args().skip(1);
    let target = values.next().ok_or_else(usage)?;
    let mut args = Args {
        target,
        config: PathBuf::from("config.toml"),
        workload: PathBuf::new(),
        feed: AsrFeedMode::Burst,
        warmup: 1,
        iterations: 1,
        output: None,
    };
    while let Some(flag) = values.next() {
        match flag.as_str() {
            "--config" => args.config = values.next().ok_or_else(usage)?.into(),
            "--workload" => args.workload = values.next().ok_or_else(usage)?.into(),
            "--feed" => {
                args.feed = match values.next().ok_or_else(usage)?.as_str() {
                    "burst" => AsrFeedMode::Burst,
                    "realtime" => AsrFeedMode::Realtime,
                    _ => return Err(usage()),
                }
            }
            "--warmup" => args.warmup = values.next().ok_or_else(usage)?.parse()?,
            "--iterations" => args.iterations = values.next().ok_or_else(usage)?.parse()?,
            "--output" => args.output = Some(values.next().ok_or_else(usage)?.into()),
            _ => return Err(usage()),
        }
    }
    if args.workload.as_os_str().is_empty() || args.warmup == 0 || args.iterations == 0 {
        return Err(usage());
    }
    Ok(args)
}

fn usage() -> anyhow::Error {
    anyhow::anyhow!(
        "usage: provider-bench-av <asr|vad|llm> --workload <json> [--config <toml>] [--feed <burst|realtime>] [--warmup N] --iterations N [--output <json>]"
    )
}

fn load_workload(path: &Path) -> anyhow::Result<Workload> {
    let workload: Workload = serde_json::from_slice(&fs::read(path)?)?;
    if workload.schema_version != 1
        || workload.items.is_empty()
        || workload.workload_version.trim().is_empty()
    {
        anyhow::bail!("workload must have schema_version=1, a version, and at least one item")
    }
    Ok(workload)
}

fn load_audio(
    workload_path: &Path,
    workload: &Workload,
) -> anyhow::Result<Vec<(String, Vec<f32>)>> {
    workload
        .items
        .iter()
        .map(|item| {
            let path = workload_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(&item.path);
            Ok((item.id.clone(), read_canonical_wav(&path)?))
        })
        .collect()
}

fn read_canonical_wav(path: &Path) -> anyhow::Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.sample_rate != 16_000
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        anyhow::bail!("{} must be PCM16 mono 16 kHz", path.display())
    }
    let samples = reader.samples::<i16>().collect::<Result<Vec<_>, _>>()?;
    if samples.is_empty() {
        anyhow::bail!("{} must not be empty", path.display())
    }
    Ok(samples
        .into_iter()
        .map(|sample| sample as f32 / i16::MAX as f32)
        .collect())
}

fn summary(values: impl Iterator<Item = f64>) -> MetricSummary {
    summarize(values)
}
fn optional_summary(values: impl Iterator<Item = f64>) -> Option<MetricSummary> {
    let values = values.collect::<Vec<_>>();
    (!values.is_empty()).then(|| summary(values.into_iter()))
}
fn publish(value: &impl serde::Serialize, output: Option<&Path>) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    if let Some(path) = output {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, json)?;
    } else {
        println!("{json}");
    }
    Ok(())
}
