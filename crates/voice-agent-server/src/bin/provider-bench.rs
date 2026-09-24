use std::{env, fs, io::Write, path::PathBuf, process::ExitCode, time::Instant};

use voice_agent_server::{
    benchmark::{BenchmarkErrorCategory, TtsBenchmarkMode, TtsBenchmarkResult, run_tts_benchmark},
    config::AppConfig,
    models::{prepare, verify_installed},
    providers::compiled_provider_registry,
};

#[derive(Clone, Debug)]
struct Args {
    mode: TtsBenchmarkMode,
    config: Option<PathBuf>,
    output: Option<PathBuf>,
    overwrite: bool,
    require_local_models: bool,
    warmup_runs: usize,
    runs: usize,
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(()) => return ExitCode::FAILURE,
    };
    let output = args.output.clone();
    let overwrite = args.overwrite;
    match run(args) {
        Ok(report) => {
            print_report(&report);
            ExitCode::SUCCESS
        }
        Err(category) => {
            if category != BenchmarkErrorCategory::OutputIo
                && let Some(path) = output
            {
                let _ = write_failure(&path, overwrite, category);
            }
            eprintln!("PROVIDER BENCHMARK: FAIL ({})", category.as_str());
            ExitCode::FAILURE
        }
    }
}

#[derive(serde::Serialize)]
struct FailureReport {
    schema_version: u8,
    status: &'static str,
    error_category: BenchmarkErrorCategory,
}

fn parse_args() -> Result<Args, ()> {
    let mut values = env::args().skip(1);
    if values.next().as_deref() != Some("tts") {
        return Err(());
    }
    let Some(mode) = values.next().as_deref().and_then(TtsBenchmarkMode::parse) else {
        return Err(());
    };
    let mut args = Args {
        mode,
        config: None,
        output: None,
        overwrite: false,
        require_local_models: false,
        warmup_runs: 1,
        runs: 5,
    };
    while let Some(flag) = values.next() {
        match flag.as_str() {
            "--config" => args.config = Some(values.next().ok_or(())?.into()),
            "--output" => args.output = Some(values.next().ok_or(())?.into()),
            "--warmup-runs" => {
                args.warmup_runs = values.next().ok_or(())?.parse().map_err(|_| ())?
            }
            "--runs" => args.runs = values.next().ok_or(())?.parse().map_err(|_| ())?,
            "--overwrite" => args.overwrite = true,
            "--require-local-models" => args.require_local_models = true,
            _ => return Err(()),
        }
    }
    Ok(args)
}

fn run(args: Args) -> Result<TtsBenchmarkResult, BenchmarkErrorCategory> {
    let overall_started = Instant::now();
    let config_path = args.config.unwrap_or_else(|| {
        env::var("VOICE_AGENT_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("config.toml"))
    });
    let config = AppConfig::load(&config_path).map_err(|_| BenchmarkErrorCategory::Config)?;
    config
        .validate()
        .map_err(|_| BenchmarkErrorCategory::Config)?;
    let registry = compiled_provider_registry();
    let factory = registry
        .tts_factory(&config.providers.tts.adapter)
        .map_err(|_| BenchmarkErrorCategory::Config)?;
    let model_identity = factory
        .model_identity(&config.providers.tts)
        .map_err(|_| BenchmarkErrorCategory::Config)?;
    let preparation_started = Instant::now();
    let model = if args.require_local_models {
        verify_installed(
            &config.deployment.model_manifest,
            &config.deployment.models.root,
            model_identity,
            factory.adapter(),
            &config.deployment,
        )
    } else {
        prepare(
            &config.deployment.model_manifest,
            &config.deployment.models.root,
            config.deployment.models.offline,
            model_identity,
            factory.adapter(),
            &config.deployment,
        )
    }
    .map_err(|_| BenchmarkErrorCategory::ModelPreparation)?;
    let model_preparation_ms = elapsed_ms(preparation_started);
    let build_started = Instant::now();
    let options = config
        .providers
        .tts
        .zerotts_onnx
        .as_ref()
        .ok_or(BenchmarkErrorCategory::Config)?;
    let provider = factory
        .build(options, &config.runtime, &model)
        .map_err(|_| BenchmarkErrorCategory::ProviderBuild)?;
    let provider_build_and_readiness_ms = elapsed_ms(build_started);
    let worker_open_started = Instant::now();
    let mut worker = provider
        .open_worker()
        .map_err(|_| BenchmarkErrorCategory::ProviderBuild)?;
    let worker_open_ms = elapsed_ms(worker_open_started);
    let mut report = run_tts_benchmark(worker.as_mut(), args.mode, args.warmup_runs, args.runs)?;
    report.adapter = Some(factory.adapter().into());
    report.model_identity = Some(model_identity.into());
    report.model_preparation_ms = Some(model_preparation_ms);
    report.provider_build_and_readiness_ms = Some(provider_build_and_readiness_ms);
    report.worker_open_ms = Some(worker_open_ms);
    report.comparison_qualified = Some(comparison_qualified(
        config.deployment.models.offline,
        args.require_local_models,
    ));
    report.overall_elapsed_ms = Some(elapsed_ms(overall_started));
    if let Some(output) = args.output {
        write_json(&output, args.overwrite, &report)?;
    }
    Ok(report)
}

fn write_json(
    path: &PathBuf,
    overwrite: bool,
    report: &TtsBenchmarkResult,
) -> Result<(), BenchmarkErrorCategory> {
    write_json_value(path, overwrite, report)
}

fn write_failure(
    path: &PathBuf,
    overwrite: bool,
    category: BenchmarkErrorCategory,
) -> Result<(), BenchmarkErrorCategory> {
    write_json_value(
        path,
        overwrite,
        &FailureReport {
            schema_version: 1,
            status: "failed",
            error_category: category,
        },
    )
}

fn write_json_value(
    path: &PathBuf,
    overwrite: bool,
    value: &impl serde::Serialize,
) -> Result<(), BenchmarkErrorCategory> {
    if path.exists() && !overwrite {
        return Err(BenchmarkErrorCategory::OutputIo);
    }
    let parent = path.parent().ok_or(BenchmarkErrorCategory::OutputIo)?;
    if !parent.as_os_str().is_empty() {
        fs::create_dir_all(parent).map_err(|_| BenchmarkErrorCategory::OutputIo)?;
    }
    let name = path.file_name().ok_or(BenchmarkErrorCategory::OutputIo)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    let payload = serde_json::to_vec_pretty(value).map_err(|_| BenchmarkErrorCategory::OutputIo)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| BenchmarkErrorCategory::OutputIo)?;
    file.write_all(&payload)
        .and_then(|_| file.sync_all())
        .map_err(|_| BenchmarkErrorCategory::OutputIo)?;
    if overwrite {
        fs::rename(&temporary, path).map_err(|_| BenchmarkErrorCategory::OutputIo)
    } else {
        fs::hard_link(&temporary, path).map_err(|_| BenchmarkErrorCategory::OutputIo)?;
        fs::remove_file(&temporary).map_err(|_| BenchmarkErrorCategory::OutputIo)
    }
}

fn print_report(report: &TtsBenchmarkResult) {
    println!("PROVIDER BENCHMARK: PASS");
    println!("mode: {:?}", report.mode);
    println!("workload_version: {}", report.workload_version);
    println!("runs: {}", report.runs);
    println!("processing_ms median: {:.3}", report.processing_ms.median);
    println!("rtf median: {:.6}", report.rtf.median);
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

const fn comparison_qualified(offline: bool, require_local_models: bool) -> bool {
    offline || require_local_models
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{BenchmarkErrorCategory, comparison_qualified, write_failure, write_json_value};

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    fn temporary_directory() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "provider-bench-output-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed),
        ))
    }

    #[test]
    fn output_without_overwrite_never_replaces_an_existing_file() {
        let directory = temporary_directory();
        fs::create_dir(&directory).unwrap();
        let path = directory.join("report.json");
        fs::write(&path, "original").unwrap();

        assert_eq!(
            write_json_value(&path, false, &serde_json::json!({"status": "passed"})),
            Err(BenchmarkErrorCategory::OutputIo)
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "original");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn output_publish_and_overwrite_are_explicit() {
        let directory = temporary_directory();
        fs::create_dir(&directory).unwrap();
        let path = directory.join("report.json");

        write_json_value(&path, false, &serde_json::json!({"version": 1})).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\n  \"version\": 1\n}");
        write_json_value(&path, true, &serde_json::json!({"version": 2})).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\n  \"version\": 2\n}");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn output_creates_a_missing_parent_directory() {
        let directory = temporary_directory();
        let path = directory.join("nested").join("report.json");

        write_json_value(&path, false, &serde_json::json!({"version": 1})).unwrap();

        assert_eq!(fs::read_to_string(path).unwrap(), "{\n  \"version\": 1\n}");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failure_output_contains_only_the_stable_category() {
        let directory = temporary_directory();
        fs::create_dir(&directory).unwrap();
        let path = directory.join("report.json");

        write_failure(&path, false, BenchmarkErrorCategory::ModelPreparation).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"schema_version\": 1,\n  \"status\": \"failed\",\n  \"error_category\": \"model_preparation\"\n}"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn comparison_requires_offline_or_explicit_local_verification() {
        assert!(!comparison_qualified(false, false));
        assert!(comparison_qualified(true, false));
        assert!(comparison_qualified(false, true));
    }
}
