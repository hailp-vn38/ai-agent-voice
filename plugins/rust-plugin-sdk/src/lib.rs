//! Shared one-shot JSONL v1 contract for untrusted Agent Job plugins.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::Path,
};

const MAX_REQUEST: u64 = 65_535;
const MAX_RESPONSE: usize = 65_536;
const MAX_ARTIFACT: usize = 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema_version: u32,
    pub request_id: String,
    pub operation: String,
    pub arguments: Value,
    pub artifact_dir: String,
    #[serde(default)]
    pub inputs: Option<Value>,
    /// Added only for packages that opt into [settings] in manifest v2.
    /// Absent for legacy jsonl-v1 plugins; never merge with user arguments.
    #[serde(default)]
    pub settings: Option<Value>,
}

// Plugin value-settings may contain credentials. A derived Debug would print
// them into stderr, which the shared runner can retain as diagnostic output.
impl std::fmt::Debug for Request {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Request")
            .field("schema_version", &self.schema_version)
            .field("request_id", &self.request_id)
            .field("operation", &self.operation)
            .field("arguments", &"[REDACTED]")
            .field("inputs", &"[REDACTED]")
            .field("settings", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Serialize)]
pub struct Artifact {
    path: String,
    media_type: String,
    size_bytes: u64,
    sha256: String,
}

#[derive(Debug)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
}
impl Failure {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub struct Outcome {
    pub result: Value,
    pub artifacts: Vec<Artifact>,
}

impl Outcome {
    pub fn new(result: Value) -> Self {
        Self {
            result,
            artifacts: vec![],
        }
    }
    pub fn attach(
        &mut self,
        dir: &str,
        name: &str,
        media_type: &str,
        bytes: &[u8],
    ) -> Result<(), Failure> {
        if !matches!(
            media_type,
            "application/json" | "text/markdown" | "text/plain"
        ) || name.is_empty()
            || name.len() > 80
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || self.artifacts.len() >= 16
            || bytes.len() > MAX_ARTIFACT
        {
            return Err(Failure::new(
                "invalid_artifact",
                "Invalid artifact name, type or size",
            ));
        }
        if !artifact_directory(dir) {
            return Err(Failure::new(
                "invalid_artifact_dir",
                "Expected an absolute artifact directory without parent traversal",
            ));
        }
        let path = Path::new(dir);
        let meta = fs::symlink_metadata(path)
            .map_err(|_| Failure::new("artifact_dir_unavailable", "Missing artifact directory"))?;
        if !meta.file_type().is_dir() || meta.file_type().is_symlink() {
            return Err(Failure::new(
                "invalid_artifact_dir",
                "Artifact directory must be a real directory",
            ));
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.join(name))
            .map_err(|_| Failure::new("artifact_write_failed", "Could not create artifact"))?;
        file.write_all(bytes)
            .map_err(|_| Failure::new("artifact_write_failed", "Could not write artifact"))?;
        self.artifacts.push(Artifact {
            path: name.into(),
            media_type: media_type.into(),
            size_bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        });
        Ok(())
    }
}

#[derive(Serialize)]
struct ErrorInfo<'a> {
    code: &'a str,
    message: &'a str,
}
#[derive(Serialize)]
struct Response<'a> {
    schema_version: u32,
    request_id: &'a str,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    artifacts: Vec<Artifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ErrorInfo<'a>>,
}

fn artifact_directory(dir: &str) -> bool {
    let path = Path::new(dir);
    path.is_absolute()
        && !path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
}

fn parse_request(input: impl Read) -> Result<Request, Failure> {
    let mut bytes = Vec::new();
    input
        .take(MAX_REQUEST + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::new("invalid_request", "Could not read stdin"))?;
    if bytes.len() as u64 > MAX_REQUEST
        || !bytes.ends_with(b"\n")
        || bytes[..bytes.len() - 1].contains(&b'\n')
    {
        return Err(Failure::new(
            "invalid_request",
            "Expected exactly one JSON line (max 65535 bytes)",
        ));
    }
    let req: Request = serde_json::from_slice(&bytes[..bytes.len() - 1])
        .map_err(|_| Failure::new("invalid_request", "Invalid request JSON"))?;
    if req.schema_version != 1
        || req.request_id.is_empty()
        || req.request_id.len() > 128
        || req.operation.is_empty()
        || !artifact_directory(&req.artifact_dir)
        || !req.arguments.is_object()
    {
        return Err(Failure::new("invalid_request", "Invalid request envelope"));
    }
    Ok(req)
}

fn emit(response: &Response<'_>) {
    let mut payload = serde_json::to_vec(response).unwrap_or_default();
    if payload.len() >= MAX_RESPONSE {
        let fallback = Response {
            schema_version: 1,
            request_id: response.request_id,
            ok: false,
            result: None,
            artifacts: vec![],
            error: Some(ErrorInfo {
                code: "response_too_large",
                message: "Result exceeds 64 KiB stdout limit",
            }),
        };
        payload = serde_json::to_vec(&fallback).unwrap_or_default();
    }
    let _ = io::stdout()
        .lock()
        .write_all(&[payload.as_slice(), b"\n"].concat());
}

fn error_message(message: &str) -> &str {
    if message.len() <= 256 {
        return message;
    }
    let end = message
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|i| *i <= 256)
        .last()
        .unwrap_or(0);
    &message[..end]
}

pub fn run(handle: impl FnOnce(&Request) -> Result<Outcome, Failure>) {
    match parse_request(io::stdin().lock()) {
        Ok(req) => match handle(&req) {
            Ok(out) => emit(&Response {
                schema_version: 1,
                request_id: &req.request_id,
                ok: true,
                result: Some(out.result),
                artifacts: out.artifacts,
                error: None,
            }),
            Err(err) => {
                let message = error_message(&err.message);
                emit(&Response {
                    schema_version: 1,
                    request_id: &req.request_id,
                    ok: false,
                    result: None,
                    artifacts: vec![],
                    error: Some(ErrorInfo {
                        code: err.code,
                        message,
                    }),
                });
            }
        },
        Err(err) => {
            let message = error_message(&err.message);
            emit(&Response {
                schema_version: 1,
                request_id: "unknown",
                ok: false,
                result: None,
                artifacts: vec![],
                error: Some(ErrorInfo {
                    code: err.code,
                    message,
                }),
            });
        }
    }
}

pub fn clipped(text: &str, maximum_chars: usize) -> (String, bool) {
    let content: String = text.chars().take(maximum_chars).collect();
    let shortened = text.chars().count() > maximum_chars;
    (content, shortened)
}

pub fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn positive_limit(value: Option<&Value>, default: usize, max: usize) -> Result<usize, Failure> {
    match value {
        None => Ok(default),
        Some(Value::Number(v)) => v
            .as_u64()
            .filter(|n| (1..=max as u64).contains(n))
            .map(|n| n as usize)
            .ok_or(Failure::new("invalid_arguments", "Invalid numeric limit")),
        _ => Err(Failure::new(
            "invalid_arguments",
            "Limit must be an integer",
        )),
    }
}

#[cfg(test)]
mod tests;
