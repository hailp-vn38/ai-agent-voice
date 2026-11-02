use crate::providers::llm::supported_agent_task_file;
use serde_json::Value;

pub(super) fn collect_file_paths(
    presentations: &[Value],
    turn_events: &[Value],
    bubbles: &[String],
) -> Vec<String> {
    fn add(text: &str, paths: &mut Vec<String>) {
        let clean = text
            .trim()
            .trim_start_matches("sandbox://")
            .trim_end_matches(['.', ',', ';', ')', ']', '\'', '"']);
        if supported_agent_task_file(clean) && !paths.iter().any(|path| path == clean) {
            paths.push(clean.to_owned());
        }
    }
    fn scan(value: &Value, paths: &mut Vec<String>) {
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    if ["path", "file_path"].contains(&key.as_str()) {
                        if let Some(path) = value.as_str() {
                            add(path, paths);
                        }
                    }
                    scan(value, paths);
                }
            }
            Value::Array(array) => {
                for value in array {
                    scan(value, paths);
                }
            }
            Value::String(text) => {
                for token in text
                    .split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`'))
                {
                    for (start, _) in token.match_indices("sandbox://") {
                        add(link_target(token, start), paths);
                    }
                }
            }
            _ => (),
        }
    }
    let mut paths = Vec::new();
    for value in presentations.iter().chain(turn_events) {
        scan(value, &mut paths);
    }
    for bubble in bubbles {
        scan(&Value::String(bubble.clone()), &mut paths);
    }
    paths
}

/// The link starting at `start`, cut at the `)` that closes a markdown link so names like
/// `report(1).md` stay whole.
fn link_target(token: &str, start: usize) -> &str {
    let rest = &token[start..];
    if !token[..start].ends_with('(') {
        return rest;
    }
    let mut depth = 1;
    for (index, c) in rest.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[..index];
                }
            }
            _ => (),
        }
    }
    rest
}

pub(super) fn mime_type(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "md" => "text/markdown",
        "txt" => "text/plain",
        "pdf" => "application/pdf",
        "csv" => "text/csv",
        "json" => "application/json",
        "html" => "text/html",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "webp" => "image/webp",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
}

pub(super) async fn download(
    lease: &super::VmOperationLease,
    paths: Vec<String>,
    limit: tokio::time::Instant,
    deadline: tokio::time::Instant,
    message: &str,
) -> Result<Vec<crate::providers::llm::AgentFile>, crate::providers::llm::AgentTaskError> {
    use super::VmError;
    use crate::providers::llm::AgentTaskError;
    use tokio::time::Instant;
    let mut downloaded = Vec::new();
    for path in paths {
        match tokio::time::timeout_at(limit, lease.client().read_file(&path, 128 * 1024 * 1024))
            .await
            .map_err(|_| {
                if Instant::now() >= deadline {
                    AgentTaskError::Timeout {
                        partial: Some(message.to_owned()),
                    }
                } else {
                    AgentTaskError::RetryLater("Muse file download still pending".into())
                }
            })? {
            Ok(bytes) => downloaded.push(crate::providers::llm::AgentFile {
                filename: path.rsplit('/').next().unwrap_or("file").to_owned(),
                mime_type: mime_type(&path).into(),
                bytes,
                source_path: path,
            }),
            Err(VmError::Transport(message)) if message.contains("404") => (),
            Err(VmError::Transport(message)) if message.contains("exceed") => {
                return Err(AgentTaskError::Rejected(
                    "Muse file exceeds artifact size limit".into(),
                ));
            }
            Err(error) => return Err(super::vm_error(error)),
        }
    }
    Ok(downloaded)
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;
