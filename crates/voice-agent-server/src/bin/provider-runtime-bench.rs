//! Offline native pool readiness measurement; run under an OS peak-RSS measurement tool.
use std::{sync::Arc, time::Instant};
use voice_agent_server::{
    config::AppConfig,
    database::secrets::EnvSecretResolver,
    providers::deployment_provider_snapshot,
    services::provider_runtime::{FactoryMaterializer, RuntimeMaterializer},
    workers::WorkerSupervisor,
};

/// Resident bytes currently mapped by this process. `/usr/bin/time -l` reports a peak for the whole
/// process lifetime; a runtime that stays `Ready` is better described by its retained size, which
/// is what this reads.
fn resident_bytes() -> u64 {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default();
    output.parse::<u64>().unwrap_or(0).saturating_mul(1024)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let client_only = args.iter().any(|value| value == "--client-only");
    let hold_ms = args
        .iter()
        .position(|value| value == "--hold-ms")
        .and_then(|index| args.get(index + 1))
        .map(|value| value.parse::<u64>())
        .transpose()?;
    if args.len() < 3 {
        return Err(
            "usage: provider-runtime-bench CONFIG KIND KEY [--client-only] [--hold-ms N]".into(),
        );
    }
    let mut config =
        AppConfig::parse_and_resolve(&args[0]).map_err(|_| "invalid qualification config")?;
    config.deployment.models.offline = true;
    let snapshot = deployment_provider_snapshot(&config, &args[1], &args[2])
        .map_err(|_| "invalid qualification provider")?;
    if !client_only && matches!(snapshot.adapter.as_str(), "openai" | "chillaudio_ws") {
        return Err("native offline providers only".into());
    }
    let supervisor = Arc::new(WorkerSupervisor::start_many(vec![], vec![]));
    // This harness measures peak memory without creating a manager reservation. Its estimate
    // field is deliberately unused by build; measured values belong in deployment config.
    let builder = FactoryMaterializer::new(
        Arc::new(config),
        Arc::new(EnvSecretResolver),
        [(snapshot.adapter.clone(), u64::MAX)].into_iter().collect(),
        supervisor,
    )?;
    let physical_capacity = builder.physical_capacity(&snapshot)?;
    // Mirror the manager: prepare inside the measured window, then hand the result to build.
    let started = Instant::now();
    let prepared = builder.prepare_artifacts(&snapshot)?;
    let prepared_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    let resource = builder.build(
        &snapshot,
        prepared,
        voice_agent_server::workers::ProviderRuntimeAdmission::new(physical_capacity, 1),
    )?;
    let ready_ms = started.elapsed().as_secs_f64() * 1000.0;
    let timings = resource.materialization_timings();
    let readiness = resource.readiness();
    let diagnostics = builder.diagnostics();
    if let Some(hold_ms) = hold_ms {
        std::thread::sleep(std::time::Duration::from_millis(hold_ms));
    }
    let ready_resident_bytes = resident_bytes();
    if !resource.unload() {
        return Err("native teardown unacknowledged".into());
    }
    println!(
        "{}",
        serde_json::json!({
            "schema_version":2,
            "adapter":snapshot.adapter,
            "artifact_prepare_ms":prepared_ms,
            "ready_ms":ready_ms,
            "total_load_ms":prepared_ms + ready_ms,
            "artifact_verify_ms":timings.artifact_verify.as_secs_f64() * 1000.0,
            "provider_contract_ms":timings.provider_contract.as_secs_f64() * 1000.0,
            "worker_session_init_ms":readiness.initialization.as_secs_f64() * 1000.0,
            "worker_warmup_ms":readiness.warmup.as_secs_f64() * 1000.0,
            "physical_replicas":physical_capacity,
            "model_preparations":diagnostics.model_preparations,
            "ready_resident_bytes":ready_resident_bytes,
            "unload_acknowledged":true,
            "client_only":client_only
        })
    );
    Ok(())
}
