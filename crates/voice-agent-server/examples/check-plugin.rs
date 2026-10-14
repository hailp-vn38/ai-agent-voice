use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{io::Read, path::PathBuf};
use voice_agent_server::{config::BackgroundConfig, services::background::plugins::qualification};

#[derive(Deserialize)]
struct Deployment {
    background: BackgroundConfig,
}

fn read(path: &str, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "file exceeds limit: {path}");
    Ok(bytes)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 2 && args[0] == "--static-only" || args.len() == 3,
        "usage: check-plugin --static-only PACKAGE.zip | PACKAGE.zip CASES.json CONFIG.toml"
    );
    let (package, cases, config) = if args[0] == "--static-only" {
        (read(&args[1], 20 * 1024 * 1024)?, Vec::new(), None)
    } else {
        let config: Deployment =
            toml::from_str(std::str::from_utf8(&read(&args[2], 1024 * 1024)?)?)
                .context("invalid deployment config")?;
        (
            read(&args[0], 20 * 1024 * 1024)?,
            serde_json::from_slice(&read(&args[1], 1024 * 1024)?)?,
            Some(config.background),
        )
    };
    let stage: PathBuf =
        std::env::temp_dir().join(format!("agent-plugin-check-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&stage)?;
    let outcome = qualification::check(&package, &cases, config.as_ref(), &stage).await;
    std::fs::remove_dir_all(&stage).context("check workspace cleanup failed")?;
    println!("{}", serde_json::to_string_pretty(&outcome?)?);
    Ok(())
}
