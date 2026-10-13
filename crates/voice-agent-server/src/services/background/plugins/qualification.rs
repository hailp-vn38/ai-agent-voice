//! Operator checks using the same package, installation, worker and artifact contracts as Jobs.
use super::{PluginError, archive, builder, dispatch, protocol, runner};
use crate::config::BackgroundConfig;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, time::Duration};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub name: String,
    pub operation: String,
    pub arguments: Value,
    #[serde(default)]
    pub expected: BTreeMap<String, Value>,
    pub expected_error: Option<String>,
    pub expected_artifacts: Option<usize>,
}

fn error(error: PluginError) -> anyhow::Error {
    anyhow::anyhow!(error.code())
}

fn schema(package: &archive::Package, path: &str) -> Result<Value> {
    let bytes = &package
        .files
        .iter()
        .find(|(name, _)| name == path)
        .context("missing operation schema")?
        .1;
    Ok(serde_json::from_slice(bytes)?)
}

fn validate_cases(package: &archive::Package, cases: &[Case]) -> Result<()> {
    ensure!(
        !cases.is_empty() && cases.len() <= 64,
        "provide 1-64 check cases"
    );
    for case in cases {
        ensure!(
            !case.name.is_empty() && case.name.len() <= 128,
            "invalid case name"
        );
        let operation = package
            .manifest
            .operations
            .get(&case.operation)
            .context("case references an undeclared operation")?;
        ensure!(
            jsonschema::validator_for(&schema(package, &operation.input_schema)?)?
                .is_valid(&case.arguments),
            "case {} violates input schema",
            case.name
        );
        ensure!(
            case.expected_error.is_none() || case.expected.is_empty(),
            "error cases cannot assert successful results"
        );
    }
    for operation in package.manifest.operations.keys() {
        ensure!(
            cases.iter().any(|case| &case.operation == operation),
            "missing check case for {operation}"
        );
    }
    Ok(())
}

/// `stage` must be a fresh operator-owned temporary directory. No installation or grants change.
pub async fn check(
    bytes: &[u8],
    cases: &[Case],
    deployment: Option<&BackgroundConfig>,
    _stage: &Path,
) -> Result<Value> {
    let package = archive::validate(bytes).map_err(error)?;
    ensure!(
        package.manifest.runtime.kind == "rust",
        "not a Rust plugin"
    );
    let instructions = &package
        .files
        .iter()
        .find(|(name, _)| name == &package.manifest.agent.instructions)
        .context("missing plugin instructions")?
        .1;
    std::str::from_utf8(instructions).context("plugin instructions must be UTF-8")?;
    for operation in package.manifest.operations.values() {
        for path in [&operation.input_schema, &operation.output_schema] {
            jsonschema::validator_for(&schema(&package, path)?)?;
        }
    }
    let mut report = json!({
        "schema_version": 1,
        "plugin_id": package.manifest.plugin.id,
        "version": package.manifest.plugin.version,
        "package_digest": package.digest,
        "package": "PASS",
        "runtime": "NOT_RUN",
        "cases": []
    });
    let Some(config) = deployment else {
        return Ok(report);
    };
    ensure!(config.limits.valid(), "invalid background limits");
    validate_cases(&package, cases)?;
    let staging_root = config.plugin_staging_dir.as_deref().context("plugin_staging_dir required for runtime checks")?;
    let stage = staging_root.join(uuid::Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&stage).await?;
    let outcome = async {
        for (name, bytes) in &package.files {
            let path = stage.join(name);
            tokio::fs::create_dir_all(path.parent().context("invalid package path")?).await?;
            tokio::fs::write(path, bytes).await?;
        }
        let installation = builder::build(config, &stage, &package).await.map_err(error)?;
        builder::verify_image(config, &installation, &package.digest, &package.manifest.runtime).await.map_err(error)?;
        let mut results = Vec::new();
        for case in cases {
            let artifacts = run_case(config, &package, &installation, case).await
                .with_context(|| format!("case {}", case.name))?;
            results.push(json!({"name":case.name,"operation":case.operation,"result":"PASS","artifact_count":artifacts}));
        }
        report["runtime"] = json!("PASS");
        report["cases"] = json!(results);
        Ok(report)
    }.await;
    tokio::fs::remove_dir_all(&stage).await.context("qualification staging cleanup failed")?;
    outcome
}

async fn run_case(
    config: &BackgroundConfig,
    package: &archive::Package,
    image: &str,
    case: &Case,
) -> Result<usize> {
    let attempt = uuid::Uuid::new_v4().to_string();
    let job = uuid::Uuid::new_v4().to_string();
    let workspace = config.plugin_attempts_dir.as_deref()
        .context("plugin_attempts_dir required for runtime checks")?
        .join(&job).join(&attempt);
    let out = workspace.join("out");
    tokio::fs::create_dir_all(&out).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o777)).await?;
    }
    let request = serde_json::to_vec(&json!({
        "schema_version": 1, "request_id": attempt, "operation": case.operation,
        "arguments": case.arguments, "artifact_dir": "/work/out"
    }))?;
    ensure!(request.len() <= 65535, "case exceeds Job request limit");
    let request = [request.as_slice(), b"\n"].concat();
    let prepared = runner::Prepared::create(config, image, &workspace, &job, &attempt, None)
        .await
        .map_err(error)?;
    let outcome = prepared.execute(
        config,
        &request,
        Duration::from_secs(30).min(Duration::from_secs(config.limits.step_timeout_seconds.into())),
        || async { Ok(()) }
    ).await.map_err(error);
    prepared.remove(config).await.map_err(error)?;
    let operation = &package.manifest.operations[&case.operation];
    let response = protocol::parse_response(
        &outcome?,
        &attempt,
        &schema(package, &operation.output_schema)?,
    )
    .map_err(error)?;
    let artifact_count = response.artifacts.len();
    if let Some(expected) = case.expected_artifacts {
        ensure!(artifact_count == expected, "unexpected artifact count");
    }
    match &case.expected_error {
        Some(code) => {
            ensure!(
                !response.ok && response.error.as_ref().is_some_and(|e| &e.code == code),
                "unexpected application error"
            );
            ensure!(
                response.artifacts.is_empty(),
                "error response declares artifacts"
            );
        }
        None => {
            ensure!(response.ok, "operation returned an application error");
            let result = response.result.context("missing result")?;
            for (pointer, expected) in &case.expected {
                ensure!(
                    result.pointer(pointer) == Some(expected),
                    "result mismatch at {pointer}"
                );
            }
            dispatch::read_artifacts(
                &out,
                response.artifacts,
                config.limits.artifact_bytes_per_job,
            )
            .await
            .map_err(error)?;
        }
    }
    tokio::fs::remove_dir_all(workspace).await?;
    Ok(artifact_count)
}

#[cfg(test)]
mod tests;
