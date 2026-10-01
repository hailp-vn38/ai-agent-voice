use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
};

use serde::{Deserialize, Serialize};

use crate::providers::tts::TtsError;

#[derive(Serialize)]
struct Request<'a> {
    id: u64,
    text: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    id: u64,
    phonemes: Option<String>,
    error: Option<String>,
}

/// One persistent child belongs to one native TTS worker. It is not shared by sessions.
pub(super) struct G2pSidecar {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl G2pSidecar {
    pub(super) fn start(executable: &Path) -> Result<Self, TtsError> {
        let mut child = Command::new(executable)
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| TtsError::Failed)?;
        let stdin = child.stdin.take().ok_or(TtsError::Failed)?;
        let stdout = child.stdout.take().ok_or(TtsError::Failed)?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }

    pub(super) fn phonemes(&mut self, text: &str) -> Result<String, TtsError> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(TtsError::Failed)?;
        serde_json::to_writer(&mut self.stdin, &Request { id, text })
            .map_err(|_| TtsError::Failed)?;
        self.stdin.write_all(b"\n").map_err(|_| TtsError::Failed)?;
        self.stdin.flush().map_err(|_| TtsError::Failed)?;

        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .map_err(|_| TtsError::Failed)?;
        let response: Response = serde_json::from_str(&line).map_err(|_| TtsError::Failed)?;
        if response.id != id || response.error.is_some() {
            return Err(TtsError::Failed);
        }
        response
            .phonemes
            .filter(|phonemes| !phonemes.is_empty())
            .ok_or(TtsError::Failed)
    }
}

impl Drop for G2pSidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
