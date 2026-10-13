use super::{
    PluginError,
    archive::{Package, Runtime},
};
use crate::config::BackgroundConfig;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::process::Command;

fn pinned_image(value: Option<&str>, prefix: &str) -> Result<String, PluginError> {
    let value = value.ok_or(PluginError::SandboxUnavailable)?;
    let digest = value
        .strip_prefix(prefix)
        .ok_or(PluginError::SandboxUnavailable)?;
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(PluginError::SandboxUnavailable);
    }
    Ok(value.to_owned())
}
fn docker_host(config: &BackgroundConfig) -> Result<&str, PluginError> {
    let host = config
        .plugin_docker_host
        .as_deref()
        .ok_or(PluginError::SandboxUnavailable)?;
    if !host.starts_with("unix:///run/user/")
        || !host.ends_with("/docker.sock")
        || host["unix:///run/user/".len()..host.len() - "/docker.sock".len()]
            .parse::<u32>()
            .is_err()
    {
        return Err(PluginError::SandboxUnavailable);
    }
    Ok(host)
}
pub(super) fn command(config: &BackgroundConfig) -> Result<Command, PluginError> {
    let mut cmd = Command::new("docker");
    cmd.env_clear()
        .env("DOCKER_HOST", docker_host(config)?)
        .env("PATH", "/usr/bin:/bin");
    cmd.kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    Ok(cmd)
}
async fn output(mut cmd: Command, seconds: u64) -> Result<String, PluginError> {
    let output = tokio::time::timeout(Duration::from_secs(seconds), cmd.output())
        .await
        .map_err(|_| PluginError::BuildTimeout)?
        .map_err(|_| PluginError::SandboxUnavailable)?;
    if !output.status.success() || output.stdout.len() > 4096 {
        return Err(PluginError::SandboxUnavailable);
    }
    String::from_utf8(output.stdout).map_err(|_| PluginError::SandboxUnavailable)
}

async fn local_image(config: &BackgroundConfig, reference: &str) -> Result<(), PluginError> {
    let mut inspect = command(config)?;
    inspect.args([
        "image",
        "inspect",
        "--format",
        "{{json .RepoDigests}}",
        reference,
    ]);
    let digests: Vec<String> = serde_json::from_str(&output(inspect, 5).await?)
        .map_err(|_| PluginError::SandboxUnavailable)?;
    let (name, sha) = reference
        .rsplit_once('@')
        .ok_or(PluginError::SandboxUnavailable)?;
    let last_slash = name.rfind('/').unwrap_or(0);
    let repository = name
        .rfind(':')
        .filter(|i| *i > last_slash)
        .map_or(name, |i| &name[..i]);
    if !digests
        .iter()
        .any(|digest| digest == &format!("{repository}@{sha}"))
    {
        return Err(PluginError::SandboxUnavailable);
    }
    Ok(())
}

fn rust_image(image: Option<&str>) -> Result<String, PluginError> {
    let image = image.ok_or(PluginError::SandboxUnavailable)?;
    let (repository, digest) = image
        .rsplit_once("@sha256:")
        .ok_or(PluginError::SandboxUnavailable)?;
    if repository.is_empty()
        || repository.starts_with('-')
        || !repository
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"./:_-".contains(&b))
        || digest.len() != 64
        || !digest.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(PluginError::SandboxUnavailable);
    }
    Ok(image.to_owned())
}

async fn platform(config: &BackgroundConfig, reference: &str) -> Result<String, PluginError> {
    let mut cmd = command(config)?;
    cmd.args([
        "image",
        "inspect",
        "--format",
        "{{.Os}}/{{.Architecture}}",
        reference,
    ]);
    let platform = output(cmd, 5).await?;
    let platform = platform.trim();
    if !platform.starts_with("linux/") || platform.len() > 40 {
        return Err(PluginError::SandboxUnavailable);
    }
    Ok(platform.to_owned())
}

pub async fn preflight(config: &BackgroundConfig, runtime: &Runtime) -> Result<(), PluginError> {
    if !cfg!(target_os = "linux") {
        return Err(PluginError::SandboxUnavailable);
    }
    let mut info = command(config)?;
    info.args(["info", "--format", "{{.OSType}} {{json .SecurityOptions}}"]);
    let info = output(info, 5).await?;
    if !info.contains("linux") || !info.contains("name=rootless") || !info.contains("name=seccomp")
    {
        return Err(PluginError::SandboxUnavailable);
    }
    if runtime.kind == "rust_container" && runtime.entrypoint != "bin/plugin" {
        let builder = rust_image(config.plugin_rust_builder_image.as_deref())?;
        let runner = rust_image(config.plugin_rust_runtime_image.as_deref())?;
        local_image(config, &builder).await?;
        local_image(config, &runner).await?;
        if platform(config, &builder).await? != platform(config, &runner).await? {
            return Err(PluginError::SandboxUnavailable);
        }
        let mut probe = command(config)?;
        probe.args([
            "run",
            "--rm",
            "--pull=never",
            "--network=none",
            "--read-only",
            "--cap-drop=ALL",
            "--security-opt=no-new-privileges",
            "--pids-limit=64",
            "--memory=128m",
            "--cpus=1",
            "--user=65532:65532",
            &builder,
            "rustc",
            "--version",
        ]);
        let result = output(probe, 15).await?;
        if !result.starts_with("rustc ") {
            return Err(PluginError::SandboxUnavailable);
        }
    } else if runtime.kind == "python_container"
        || (runtime.kind == "rust_container" && runtime.entrypoint == "bin/plugin")
    {
        let python = pinned_image(
            config.plugin_python_image.as_deref(),
            "python:3.12-slim@sha256:",
        )?;
        let uv = pinned_image(
            config.plugin_uv_image.as_deref(),
            "ghcr.io/astral-sh/uv@sha256:",
        )?;
        local_image(config, &python).await?;
        local_image(config, &uv).await?;
        let mut probe = command(config)?;
        probe.args([
            "run", "--rm", "--pull=never", "--network=none", "--read-only",
            "--cap-drop=ALL", "--security-opt=no-new-privileges", "--pids-limit=64",
            "--memory=128m", "--cpus=1", "--user=65532:65532", &python,
            "python", "-c",
            "import os; assert os.getuid()==65532; assert not os.access('/etc', os.W_OK); print('sandbox_ready')",
        ]);
        if output(probe, 15).await?.trim() != "sandbox_ready" {
            return Err(PluginError::SandboxUnavailable);
        }
    } else {
        return Err(PluginError::InvalidPackage);
    }
    Ok(())
}

pub async fn build(
    config: &BackgroundConfig,
    stage: &Path,
    package: &Package,
) -> Result<String, PluginError> {
    let (dockerfile, builder_ref, secondary_label, secondary_ref) = if package.manifest.runtime.kind
        == "rust_container"
        && package.manifest.runtime.entrypoint != "bin/plugin"
    {
        let builder = rust_image(config.plugin_rust_builder_image.as_deref())?;
        let runtime = rust_image(config.plugin_rust_runtime_image.as_deref())?;
        let name = &package.manifest.runtime.entrypoint; // ZIP validator already constrained this name.
        (
            format!(
                "FROM {builder} AS build\nWORKDIR /plugin\nCOPY . /plugin\nRUN --network=none cargo build --release --locked --offline --bin {name}\nFROM {runtime}\nCOPY --from=build /plugin/target/release/{name} /worker\nUSER 65532:65532\nENTRYPOINT [\"/worker\"]\n"
            ),
            builder,
            "agent.plugin.rust.runtime",
            runtime,
        )
    } else {
        let python = pinned_image(
            config.plugin_python_image.as_deref(),
            "python:3.12-slim@sha256:",
        )?;
        let uv = pinned_image(
            config.plugin_uv_image.as_deref(),
            "ghcr.io/astral-sh/uv@sha256:",
        )?;
        (
            if package.manifest.runtime.kind == "rust_container" {
                format!(
                    "FROM {python}\nWORKDIR /plugin\nCOPY bin/plugin /plugin/bin/plugin\nRUN chmod 0555 /plugin/bin/plugin && mkdir -p /work/out\nUSER 65532:65532\nENTRYPOINT [\"/plugin/bin/plugin\"]\n"
                )
            } else {
                format!(
                    "FROM {python}\nCOPY --from={uv} /uv /usr/local/bin/uv\nWORKDIR /plugin\nCOPY . /plugin\nRUN --network=none uv sync --locked --offline --no-dev --no-install-project\nRUN mkdir -p /work/out\nENV PYTHONDONTWRITEBYTECODE=1\nUSER 65532:65532\nENTRYPOINT [\"/plugin/.venv/bin/python\",\"/plugin/src/main.py\"]\n"
                )
            },
            python,
            "agent.plugin.uv",
            uv,
        )
    };
    let recipe = stage.join(".agent-plugin.Dockerfile");
    tokio::fs::write(&recipe, dockerfile)
        .await
        .map_err(|_| PluginError::StorageUnavailable)?;
    let tag = format!("agent-plugin-build-{}", uuid::Uuid::new_v4());
    let mut build = command(config)?;
    build.stdout(Stdio::null());
    build
        .args([
            "build",
            "--network=none",
            "--pull=false",
            "--no-cache",
            "--label",
        ])
        .arg(format!("agent.plugin.package={}", package.digest))
        .arg("--label")
        .arg(format!("agent.plugin.base={builder_ref}"))
        .arg("--label")
        .arg(format!("{secondary_label}={secondary_ref}"))
        .arg("--label")
        .arg(format!(
            "agent.plugin.runtime={}",
            package.manifest.runtime.kind
        ))
        .arg("-f")
        .arg(&recipe)
        .args(["-t", &tag])
        .arg(stage);
    let status = tokio::time::timeout(Duration::from_secs(120), build.status())
        .await
        .map_err(|_| PluginError::BuildTimeout)?
        .map_err(|_| PluginError::ImageBuildFailed)?;
    if !status.success() {
        return Err(PluginError::ImageBuildFailed);
    }
    let mut inspect = command(config)?;
    inspect.args(["image", "inspect", "--format", "{{.Id}}", &tag]);
    let id = output(inspect, 5)
        .await
        .map_err(|_| PluginError::ImageBuildFailed)?;
    let id = id.trim().to_owned();
    if !id.starts_with("sha256:")
        || id.len() != 71
        || !id[7..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(PluginError::ImageBuildFailed);
    }
    Ok(id)
}

pub async fn verify_image(
    config: &BackgroundConfig,
    digest: &str,
    package_digest: &str,
    runtime: &Runtime,
) -> Result<(), PluginError> {
    if !digest.starts_with("sha256:")
        || digest.len() != 71
        || !digest[7..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(PluginError::SandboxUnavailable);
    }
    let mut inspect = command(config)?;
    inspect.args(["image", "inspect", "--format", "{{.Id}}", digest]);
    if output(inspect, 5).await?.trim() != digest {
        return Err(PluginError::SandboxUnavailable);
    }
    let mut labels = command(config)?;
    labels.args([
        "image",
        "inspect",
        "--format",
        "{{json .Config.Labels}}",
        digest,
    ]);
    let json: serde_json::Value = serde_json::from_str(&output(labels, 5).await?)
        .map_err(|_| PluginError::SandboxUnavailable)?;
    if runtime.kind == "rust_container" && json["agent.plugin.runtime"] != "rust_container" {
        return Err(PluginError::SandboxUnavailable);
    }
    if json["agent.plugin.package"] != package_digest {
        return Err(PluginError::SandboxUnavailable);
    }
    if runtime.kind == "rust_container" && runtime.entrypoint != "bin/plugin" {
        let builder = rust_image(config.plugin_rust_builder_image.as_deref())?;
        let runner = rust_image(config.plugin_rust_runtime_image.as_deref())?;
        if json["agent.plugin.base"] != builder
            || json["agent.plugin.rust.runtime"] != runner
            || platform(config, &builder).await? != platform(config, &runner).await?
            || platform(config, digest).await? != platform(config, &runner).await?
        {
            return Err(PluginError::SandboxUnavailable);
        }
    } else if runtime.kind == "python_container"
        || (runtime.kind == "rust_container" && runtime.entrypoint == "bin/plugin")
    {
        if json["agent.plugin.base"]
            != pinned_image(
                config.plugin_python_image.as_deref(),
                "python:3.12-slim@sha256:",
            )?
            || json["agent.plugin.uv"]
                != pinned_image(
                    config.plugin_uv_image.as_deref(),
                    "ghcr.io/astral-sh/uv@sha256:",
                )?
        {
            return Err(PluginError::SandboxUnavailable);
        }
    } else {
        return Err(PluginError::InvalidPackage);
    }
    Ok(())
}
