//! Offline native pool readiness measurement; run under an OS peak-RSS measurement tool.
use std::{sync::Arc, time::Instant};
use voice_agent_server::{
    config::AppConfig,
    database::secrets::EnvSecretResolver,
    providers::deployment_provider_snapshot,
    services::provider_runtime::{FactoryMaterializer, RuntimeMaterializer},
    workers::WorkerSupervisor,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let client_only = args.len() == 4 && args[3] == "--client-only";
    if args.len() != 3 && !client_only {
        return Err("usage: provider-runtime-bench CONFIG KIND KEY [--client-only]".into());
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
    let capacity = builder.logical_capacity(&snapshot)?;
    let started = Instant::now();
    let resource = builder.build(
        &snapshot,
        voice_agent_server::workers::ProviderRuntimeAdmission::new(capacity, 1),
    )?;
    let ready_ms = started.elapsed().as_secs_f64() * 1000.0;
    if !resource.unload() {
        return Err("native teardown unacknowledged".into());
    }
    println!(
        "{}",
        serde_json::json!({"schema_version":1,"adapter":snapshot.adapter,"ready_ms":ready_ms,"unload_acknowledged":true,"client_only":client_only})
    );
    Ok(())
}
