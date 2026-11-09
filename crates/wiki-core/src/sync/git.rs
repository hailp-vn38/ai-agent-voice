//! Git process invocation, environment, and error classification.

use super::{GitAuth, SyncConfig, SyncError};
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(super) fn run_git(
    root: &Path,
    args: &[&str],
    config: &SyncConfig,
    git_binary: Option<&Path>,
    classify: bool,
) -> Result<GitOutput, SyncError> {
    let Some(git_binary) = git_binary else {
        return Err(SyncError::GitMissing);
    };
    let mut command = Command::new(git_binary);
    command
        .current_dir(root)
        .arg("-c")
        .arg(format!("safe.directory={}", root.display()))
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Git global configuration is disabled for a deploy key so no ambient credential helper is consulted.
    if let Some((ssh, global_config, no_system_config)) = ssh_environment(&config.auth) {
        command
            .env("GIT_SSH_COMMAND", ssh)
            .env("GIT_CONFIG_GLOBAL", global_config)
            .env("GIT_CONFIG_NOSYSTEM", no_system_config)
            .env_remove("HOME");
    }
    if let Some(author) = &config.author {
        command
            .env("GIT_AUTHOR_NAME", &author.name)
            .env("GIT_AUTHOR_EMAIL", &author.email)
            .env("GIT_COMMITTER_NAME", &author.name)
            .env("GIT_COMMITTER_EMAIL", &author.email);
    }
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            SyncError::GitMissing
        } else {
            SyncError::Git(error.to_string())
        }
    })?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::Read::read_to_end(&mut std::io::BufReader::new(stdout), &mut bytes);
        bytes
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::Read::read_to_end(&mut std::io::BufReader::new(stderr), &mut bytes);
        bytes
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| SyncError::Git(error.to_string()))?
        {
            break status;
        }
        if started.elapsed() >= config.command_timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(SyncError::Timeout {
                command: args.first().copied().unwrap_or("git").into(),
            });
        }
        thread::sleep(Duration::from_millis(10));
    };
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = sanitize(&String::from_utf8_lossy(
        &stderr_reader.join().unwrap_or_default(),
    ));
    if !status.success() {
        let message = if stderr.is_empty() {
            "git exited unsuccessfully".into()
        } else {
            stderr
        };
        let lowered = message.to_ascii_lowercase();
        let error = if lowered.contains("authentication failed")
            || lowered.contains("permission denied (publickey)")
            || lowered.contains("could not read username")
            || lowered.contains("access denied")
        {
            SyncError::Auth(message)
        } else if lowered.contains("could not resolve host")
            || lowered.contains("network is unreachable")
            || lowered.contains("connection timed out")
            || lowered.contains("failed to connect")
        {
            SyncError::Network(message)
        } else {
            SyncError::Git(message)
        };
        let _ = classify;
        return Err(error);
    }
    Ok(GitOutput { stdout })
}

pub(super) struct GitOutput {
    pub(super) stdout: Vec<u8>,
}
impl GitOutput {
    pub(super) fn stdout_string(&self) -> Result<String, SyncError> {
        String::from_utf8(self.stdout.clone()).map_err(|error| SyncError::Git(error.to_string()))
    }
}

#[cfg(unix)]
pub(super) fn os_string(bytes: &[u8]) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(bytes.to_vec())
}
#[cfg(not(unix))]
pub(super) fn os_string(bytes: &[u8]) -> std::ffi::OsString {
    std::ffi::OsString::from(String::from_utf8_lossy(bytes).into_owned())
}
pub(super) fn hostname() -> String {
    #[cfg(unix)]
    {
        let mut bytes = [0u8; 256];
        let result = unsafe { libc::gethostname(bytes.as_mut_ptr().cast(), bytes.len()) };
        if result == 0 {
            let end = bytes
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(bytes.len());
            let value = String::from_utf8_lossy(&bytes[..end]).trim().to_string();
            if !value.is_empty() {
                return value;
            }
        }
    }
    std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown-host".into())
}
pub(super) fn is_unsafe_remote(remote: &str) -> bool {
    let has_userinfo = remote.split_once("://").is_some_and(|(_, rest)| {
        rest.split('/')
            .next()
            .is_some_and(|authority| authority.contains('@'))
    });
    let lower = remote.to_ascii_lowercase();
    has_userinfo
        || ["token=", "password=", "passwd=", "access_token=", "auth="]
            .iter()
            .any(|marker| lower.contains(marker))
}
pub(super) fn safe_remote(remote: &str) -> String {
    let redacted = if let Some((base, query)) = remote.split_once('?') {
        let lower = query.to_ascii_lowercase();
        if ["token=", "password=", "passwd=", "access_token=", "auth="]
            .iter()
            .any(|marker| lower.contains(marker))
        {
            format!("{base}?[redacted]")
        } else {
            remote.to_string()
        }
    } else {
        remote.to_string()
    };
    redacted
        .split_once("://")
        .map(|(scheme, rest)| {
            format!(
                "{scheme}://{}",
                rest.rsplit_once('@').map(|(_, host)| host).unwrap_or(rest)
            )
        })
        .unwrap_or_else(|| "remote URL contains credentials".into())
}
pub(super) fn sanitize(message: &str) -> String {
    let mut output = String::new();
    for token in message.split_whitespace() {
        if !output.is_empty() {
            output.push(' ');
        }
        let token = token.trim_matches(|ch: char| matches!(ch, ',' | ';' | ')' | '(' | '"' | '\''));
        if token.contains("://") && token.contains('@') {
            output.push_str(&safe_remote(token));
        } else if token.to_ascii_lowercase().contains("token=")
            || token.to_ascii_lowercase().contains("password=")
        {
            output.push_str("[redacted]");
        } else {
            output.push_str(token);
        }
    }
    output
}
fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

pub(super) fn ssh_command(key: &Path, known_hosts: &Path) -> String {
    format!(
        "ssh -i {} -o IdentitiesOnly=yes -o UserKnownHostsFile={} -o StrictHostKeyChecking=yes",
        shell_quote(key),
        shell_quote(known_hosts)
    )
}

pub(super) fn ssh_environment(auth: &GitAuth) -> Option<(String, &'static str, &'static str)> {
    match auth {
        GitAuth::System => None,
        GitAuth::SshKey { key, known_hosts } => {
            Some((ssh_command(key, known_hosts), "/dev/null", "1"))
        }
    }
}

pub(super) fn find_git(search_path: Option<&OsStr>) -> Option<PathBuf> {
    let path = search_path?;
    std::env::split_paths(path)
        .map(|directory| directory.join("git"))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}
