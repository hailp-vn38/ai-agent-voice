use crate::{CommandError, DesktopState, claude_config, locked, mcp_server, start_mcp_server};
use std::sync::MutexGuard;
use tauri::{Manager, State};
use wiki_mcp::Access;

type ServerSlot<'a> = MutexGuard<'a, Option<mcp_server::McpServerRuntime>>;

/// The running MCP server, or why it is not running.
fn running(state: &DesktopState) -> Result<ServerSlot<'_>, CommandError> {
    let slot = locked(&state.mcp_server, "MCP server")?;
    if slot.is_some() {
        return Ok(slot);
    }
    let message = locked(&state.mcp_error, "MCP error")?
        .clone()
        .map(|error| format!("MCP server is not running: {error}"))
        .unwrap_or_else(|| "Open a vault before enabling MCP".into());
    Err(CommandError::NotFound { message })
}

fn info(slot: &ServerSlot<'_>) -> Result<mcp_server::McpInfo, CommandError> {
    slot.as_ref()
        .expect("checked by running()")
        .info()
        .map_err(CommandError::io)
}

#[tauri::command]
pub(super) fn mcp_info(
    state: State<'_, DesktopState>,
) -> Result<mcp_server::McpInfo, CommandError> {
    info(&running(&state)?)
}

#[tauri::command]
pub(super) fn mcp_set_allow_write(
    state: State<'_, DesktopState>,
    allow_write: bool,
) -> Result<mcp_server::McpInfo, CommandError> {
    let slot = running(&state)?;
    slot.as_ref()
        .expect("checked by running()")
        .set_allow_write(allow_write)
        .map_err(CommandError::io)?;
    locked(&state.settings, "settings")?.mcp_allow_write = allow_write;
    state.persist()?;
    info(&slot)
}

/// Binds the new port before stopping the old server, so a busy port leaves
/// the current server running.
#[tauri::command]
pub(super) fn mcp_set_port(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    port: u16,
) -> Result<mcp_server::McpInfo, CommandError> {
    if port == 0 {
        return Err(CommandError::InvalidInput {
            message: "port must be between 1 and 65535".into(),
        });
    }
    if info(&running(&state)?)?.port != port {
        start_mcp_server(app, &state, Some(port))?;
    }
    info(&running(&state)?)
}

#[tauri::command]
pub(super) fn mcp_rotate_token(
    state: State<'_, DesktopState>,
    access: Access,
) -> Result<mcp_server::McpInfo, CommandError> {
    let slot = running(&state)?;
    slot.as_ref()
        .expect("checked by running()")
        .rotate_token(access)
        .map_err(CommandError::io)?;
    info(&slot)
}

#[tauri::command]
pub(super) fn mcp_claude_config_preview(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
) -> Result<claude_config::Preview, CommandError> {
    let server = running(&state)?
        .as_ref()
        .expect("checked by running()")
        .stdio_server()
        .map_err(CommandError::io)?;
    let path = claude_config::config_path(&app.path().home_dir().map_err(CommandError::io)?);
    let preview = claude_config::preview(path, server).map_err(CommandError::io)?;
    *locked(&state.claude_preview, "Claude preview")? = Some(preview.clone());
    Ok(preview)
}

/// Writes the config the user last previewed, never a freshly computed one.
#[tauri::command]
pub(super) fn mcp_claude_config_install(
    state: State<'_, DesktopState>,
) -> Result<(), CommandError> {
    let preview = locked(&state.claude_preview, "Claude preview")?
        .take()
        .ok_or_else(|| CommandError::NotFound {
            message: "Preview the Claude Desktop config before installing it".into(),
        })?;
    claude_config::install(&preview).map_err(|error| match error {
        claude_config::InstallError::Changed => CommandError::Conflict {
            message: "Claude Desktop config changed after the preview; preview it again".into(),
            current: String::new(),
        },
        claude_config::InstallError::Io(message) => CommandError::Io { message },
    })
}
