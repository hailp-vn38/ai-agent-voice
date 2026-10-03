//! Machine-readable, nonce-bound listener handoff for the qualification harness.

use serde::Serialize;
use std::{
    env,
    fs::OpenOptions,
    io::Write,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};
use uuid::Uuid;

pub const BOUND_ADDRESS_FILE: &str = "VOICE_AGENT_BOUND_ADDRESS_FILE";
pub const STARTUP_NONCE: &str = "VOICE_AGENT_STARTUP_NONCE";

pub struct StartupHandshake {
    path: PathBuf,
    nonce: String,
}

#[derive(Serialize)]
struct Artifact<'a> {
    version: u8,
    startup_nonce: &'a str,
    pid: u32,
    address: SocketAddr,
}

impl StartupHandshake {
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let path = env::var_os(BOUND_ADDRESS_FILE);
        let nonce = env::var(STARTUP_NONCE).ok();
        match (path, nonce) {
            (None, None) => Ok(None),
            (Some(_), None) | (None, Some(_)) => anyhow::bail!("startup_handshake_invalid_pair"),
            (Some(path), Some(nonce)) => {
                let path = PathBuf::from(path);
                if !path.is_absolute() || path.parent().is_none_or(|parent| !parent.is_dir()) {
                    anyhow::bail!("startup_handshake_invalid_path");
                }
                if Uuid::parse_str(&nonce)
                    .ok()
                    .is_none_or(|id| id.to_string() != nonce)
                {
                    anyhow::bail!("startup_handshake_invalid_nonce");
                }
                Ok(Some(Self { path, nonce }))
            }
        }
    }

    pub fn bind_address(&self) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
    }

    pub fn publish(&self, address: SocketAddr) -> anyhow::Result<()> {
        if self.path.exists() {
            anyhow::bail!("startup_handshake_artifact_exists");
        }
        if !address.ip().is_loopback() || address.port() == 0 {
            anyhow::bail!("startup_handshake_invalid_address");
        }
        let parent = self.path.parent().expect("validated parent");
        let temp = parent.join(format!(
            ".{}.{}.tmp",
            self.path
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or("bound-address"),
            Uuid::new_v4()
        ));
        let result = (|| -> anyhow::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            serde_json::to_writer(
                &mut file,
                &Artifact {
                    version: 1,
                    startup_nonce: &self.nonce,
                    pid: std::process::id(),
                    address,
                },
            )?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            // `rename` replaces an existing destination on Unix, which would turn a race into an
            // overwrite. A hard link is create-new at the destination and stays within this
            // directory/filesystem; only then can the temporary name be removed.
            std::fs::hard_link(&temp, &self.path)?;
            std::fs::remove_file(&temp)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binding_is_literal_loopback_only_when_handshake_is_enabled() {
        let handshake = StartupHandshake {
            path: PathBuf::from("/tmp/voice-bound.json"),
            nonce: "7df916d6-a7d5-4bea-b1ca-5db86c09fa7e".into(),
        };
        assert_eq!(handshake.bind_address(), "127.0.0.1:0".parse().unwrap());
    }

    #[test]
    fn publishing_is_create_new_and_includes_the_nonce_and_bound_address() {
        let parent = std::env::temp_dir().join(format!("voice-handshake-{}", Uuid::new_v4()));
        std::fs::create_dir(&parent).unwrap();
        let path = parent.join("bound-address.json");
        let handshake = StartupHandshake {
            path: path.clone(),
            nonce: "7df916d6-a7d5-4bea-b1ca-5db86c09fa7e".into(),
        };
        handshake
            .publish("127.0.0.1:43210".parse().unwrap())
            .unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["startup_nonce"], handshake.nonce);
        assert_eq!(value["address"], "127.0.0.1:43210");
        assert!(
            handshake
                .publish("127.0.0.1:43211".parse().unwrap())
                .is_err()
        );
        std::fs::remove_dir_all(parent).unwrap();
    }
}
