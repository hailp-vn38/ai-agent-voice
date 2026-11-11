//! Adds the Wiki stdio server to Claude Desktop's config, behind a preview.
//!
//! The preview is kept by the shell and install writes exactly that preview,
//! and only if the file still holds what the user was shown.

use serde::Serialize;
use std::path::{Path, PathBuf};

const SERVER_NAME: &str = "wiki";

pub fn config_path(home: &Path) -> PathBuf {
    home.join("Library/Application Support/Claude/claude_desktop_config.json")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffKind {
    Same,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub path: PathBuf,
    /// File content when previewed; `None` when the file did not exist.
    #[serde(skip)]
    before: Option<String>,
    #[serde(skip)]
    after: String,
    pub diff: Vec<DiffLine>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum InstallError {
    /// The file changed after the preview was shown.
    Changed,
    Io(String),
}

/// Builds the preview of adding `server` (the `mcpServers.wiki` entry).
pub fn preview(path: PathBuf, server: serde_json::Value) -> Result<Preview, String> {
    let before = read_existing(&path)?;
    let after = merge(before.as_deref(), server)?;
    let diff = diff_lines(before.as_deref().unwrap_or(""), &after);
    Ok(Preview {
        path,
        before,
        after,
        diff,
    })
}

pub fn install(preview: &Preview) -> Result<(), InstallError> {
    if read_existing(&preview.path).map_err(InstallError::Io)? != preview.before {
        return Err(InstallError::Changed);
    }
    let io = |error: std::io::Error| InstallError::Io(error.to_string());
    let parent = preview
        .path
        .parent()
        .ok_or_else(|| InstallError::Io("Claude config has no parent directory".into()))?;
    std::fs::create_dir_all(parent).map_err(io)?;
    let temporary = preview.path.with_extension("json.tmp");
    std::fs::write(&temporary, &preview.after).map_err(io)?;
    std::fs::rename(temporary, &preview.path).map_err(io)
}

fn read_existing(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn merge(current: Option<&str>, server: serde_json::Value) -> Result<String, String> {
    let mut config: serde_json::Value = match current {
        Some(text) => serde_json::from_str(text)
            .map_err(|error| format!("Claude config is not valid JSON: {error}"))?,
        None => serde_json::json!({}),
    };
    let root = config
        .as_object_mut()
        .ok_or("Claude config must contain a JSON object")?;
    let servers = root
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    if !servers.is_object() {
        *servers = serde_json::json!({});
    }
    servers
        .as_object_mut()
        .expect("mcpServers is an object")
        .insert(SERVER_NAME.into(), server);
    let mut text = serde_json::to_string_pretty(&config).map_err(|error| error.to_string())?;
    text.push('\n');
    Ok(text)
}

/// Line diff by longest common subsequence; config files are small.
fn diff_lines(before: &str, after: &str) -> Vec<DiffLine> {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    let mut common = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            common[i][j] = if old[i] == new[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let line = |kind, text: &str| DiffLine {
        kind,
        text: text.to_owned(),
    };
    let (mut i, mut j, mut diff) = (0, 0, Vec::new());
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            diff.push(line(DiffKind::Same, old[i]));
            i += 1;
            j += 1;
        } else if j < new.len() && (i == old.len() || common[i][j + 1] >= common[i + 1][j]) {
            diff.push(line(DiffKind::Added, new[j]));
            j += 1;
        } else {
            diff.push(line(DiffKind::Removed, old[i]));
            i += 1;
        }
    }
    diff
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn server() -> serde_json::Value {
        json!({"command": "/Applications/Wiki.app/Contents/MacOS/wiki-mcp", "args": ["--vault", "/v"]})
    }

    #[test]
    fn preview_keeps_other_servers_and_marks_only_new_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        let existing =
            "{\n  \"mcpServers\": {\n    \"other\": {\n      \"command\": \"x\"\n    }\n  }\n}\n";
        std::fs::write(&path, existing).unwrap();

        let preview = preview(path.clone(), server()).unwrap();
        let after: serde_json::Value = serde_json::from_str(&preview.after).unwrap();
        assert_eq!(after["mcpServers"]["other"]["command"], "x");
        assert_eq!(after["mcpServers"]["wiki"], server());
        assert!(
            preview
                .diff
                .iter()
                .any(|line| line.kind == DiffKind::Added && line.text.contains("\"wiki\""))
        );
        assert!(
            preview
                .diff
                .iter()
                .filter(|line| line.kind == DiffKind::Removed)
                .all(|line| !line.text.contains("other"))
        );
    }

    #[test]
    fn install_writes_exactly_the_preview() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Claude/claude_desktop_config.json");
        let preview = preview(path.clone(), server()).unwrap();
        assert!(preview.diff.iter().all(|line| line.kind == DiffKind::Added));

        install(&preview).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), preview.after);
    }

    #[test]
    fn install_refuses_when_the_file_changed_after_preview() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        std::fs::write(&path, "{}").unwrap();
        let preview = preview(path.clone(), server()).unwrap();

        std::fs::write(&path, "{\"mcpServers\": {\"edited\": {}}}").unwrap();
        assert_eq!(install(&preview), Err(InstallError::Changed));
        assert!(std::fs::read_to_string(&path).unwrap().contains("edited"));
    }
}
