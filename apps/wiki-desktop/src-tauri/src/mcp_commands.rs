use crate::start_mcp_server;
use crate::{CommandError, DesktopState, mcp_server};
use serde::Serialize;
use std::path::PathBuf;
use tauri::{Manager, State};

fn mcp_server<'a>(
    state: &'a DesktopState,
) -> Result<std::sync::MutexGuard<'a, Option<mcp_server::McpServerRuntime>>, CommandError> {
    state.mcp_server.lock().map_err(|_| CommandError::Io {
        message: "MCP server lock poisoned".into(),
    })
}

#[tauri::command]
pub(super) fn mcp_info(
    state: State<'_, DesktopState>,
) -> Result<mcp_server::McpInfo, CommandError> {
    mcp_server(&state)?
        .as_ref()
        .ok_or_else(|| CommandError::NotFound {
            message: "Open a vault before enabling MCP".into(),
        })?
        .info()
        .map_err(|message| CommandError::Io { message })
}

#[tauri::command]
pub(super) fn mcp_set_allow_write(
    state: State<'_, DesktopState>,
    allow_write: bool,
) -> Result<mcp_server::McpInfo, CommandError> {
    let mut server = mcp_server(&state)?;
    let server = server.as_mut().ok_or_else(|| CommandError::NotFound {
        message: "Open a vault before enabling MCP".into(),
    })?;
    server
        .set_allow_write(allow_write)
        .map_err(|message| CommandError::Io { message })?;
    state
        .settings
        .lock()
        .map_err(|_| CommandError::Io {
            message: "settings lock poisoned".into(),
        })?
        .mcp_allow_write = allow_write;
    state.persist()?;
    server
        .info()
        .map_err(|message| CommandError::Io { message })
}

#[tauri::command]
pub(super) fn mcp_set_port(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    port: u16,
) -> Result<mcp_server::McpInfo, CommandError> {
    if port == 0 {
        return Err(CommandError::InvalidPath {
            message: "port must be between 1 and 65535".into(),
        });
    }
    let current = mcp_server(&state)?
        .as_ref()
        .ok_or_else(|| CommandError::NotFound {
            message: "Open a vault before enabling MCP".into(),
        })?
        .info()
        .map_err(|message| CommandError::Io { message })?;
    if current.port == port {
        return Ok(current);
    }
    let old_port = {
        let mut settings = state.settings.lock().map_err(|_| CommandError::Io {
            message: "settings lock poisoned".into(),
        })?;
        let previous = settings.mcp_port;
        settings.mcp_port = Some(port);
        previous
    };
    state.persist()?;
    if let Err(error) = start_mcp_server(app.clone(), &state) {
        if let Ok(mut settings) = state.settings.lock() {
            settings.mcp_port = old_port;
        }
        let _ = state.persist();
        let _ = start_mcp_server(app, &state);
        return Err(error);
    }
    mcp_server(&state)?
        .as_ref()
        .ok_or_else(|| CommandError::NotFound {
            message: "MCP server did not start".into(),
        })?
        .info()
        .map_err(|message| CommandError::Io { message })
}

#[tauri::command]
pub(super) fn mcp_rotate_token(
    state: State<'_, DesktopState>,
    access: String,
) -> Result<mcp_server::McpInfo, CommandError> {
    let server = mcp_server(&state)?;
    let server = server.as_ref().ok_or_else(|| CommandError::NotFound {
        message: "Open a vault before enabling MCP".into(),
    })?;
    let write = match access.as_str() {
        "read" => false,
        "write" => true,
        _ => {
            return Err(CommandError::InvalidPath {
                message: "access must be read or write".into(),
            });
        }
    };
    server
        .rotate_token(write)
        .map_err(|message| CommandError::Io { message })?;
    server
        .info()
        .map_err(|message| CommandError::Io { message })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClaudeConfigPreview {
    path: String,
    before: String,
    after: String,
}

fn claude_config(
    app: &tauri::AppHandle,
    state: &DesktopState,
) -> Result<(PathBuf, serde_json::Value), CommandError> {
    let path = app
        .path()
        .home_dir()
        .map_err(|error| CommandError::Io {
            message: error.to_string(),
        })?
        .join("Library/Application Support/Claude/claude_desktop_config.json");
    let current = if path.exists() {
        serde_json::from_slice(&std::fs::read(&path).map_err(|error| CommandError::Io {
            message: error.to_string(),
        })?)
        .map_err(|error| CommandError::Io {
            message: format!("Claude config is not valid JSON: {error}"),
        })?
    } else {
        serde_json::json!({})
    };
    let mcp = mcp_server(state)?
        .as_ref()
        .ok_or_else(|| CommandError::NotFound {
            message: "Open a vault before enabling MCP".into(),
        })?
        .info()
        .map_err(|message| CommandError::Io { message })?;
    let snippet: serde_json::Value =
        serde_json::from_str(&mcp.stdio_config).map_err(|error| CommandError::Io {
            message: error.to_string(),
        })?;
    let mut updated = current;
    let root = updated.as_object_mut().ok_or_else(|| CommandError::Io {
        message: "Claude config must contain a JSON object".into(),
    })?;
    if !root
        .get("mcpServers")
        .is_some_and(serde_json::Value::is_object)
    {
        root.insert("mcpServers".into(), serde_json::json!({}));
    }
    root.get_mut("mcpServers")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| CommandError::Io {
            message: "Claude mcpServers must be a JSON object".into(),
        })?
        .insert("wiki".into(), snippet["mcpServers"]["wiki"].clone());
    Ok((path, updated))
}

#[tauri::command]
pub(super) fn mcp_claude_config_preview(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
) -> Result<ClaudeConfigPreview, CommandError> {
    let (path, updated) = claude_config(&app, &state)?;
    let before = if path.exists() {
        std::fs::read_to_string(&path).map_err(|error| CommandError::Io {
            message: error.to_string(),
        })?
    } else {
        "(file does not exist yet)".into()
    };
    let after = serde_json::to_string_pretty(&updated).map_err(|error| CommandError::Io {
        message: error.to_string(),
    })?;
    Ok(ClaudeConfigPreview {
        path: path.to_string_lossy().into_owned(),
        before,
        after,
    })
}

#[tauri::command]
pub(super) fn mcp_claude_config_install(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    let (path, updated) = claude_config(&app, &state)?;
    let bytes = serde_json::to_vec_pretty(&updated).map_err(|error| CommandError::Io {
        message: error.to_string(),
    })?;
    let parent = path.parent().ok_or_else(|| CommandError::Io {
        message: "Claude config has no parent directory".into(),
    })?;
    std::fs::create_dir_all(parent).map_err(|error| CommandError::Io {
        message: error.to_string(),
    })?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes).map_err(|error| CommandError::Io {
        message: error.to_string(),
    })?;
    std::fs::rename(temporary, path).map_err(|error| CommandError::Io {
        message: error.to_string(),
    })?;
    Ok(())
}
