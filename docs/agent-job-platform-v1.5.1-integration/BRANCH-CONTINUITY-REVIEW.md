# Branch continuity — integration/voice-reminders-direct-tts

Checked against the GitHub repository `hailp-vn38/ai-agent-voice`, branch `integration/voice-reminders-direct-tts` (2026-10-13). The branch already contains Device Reminders + Direct Speech and ADR 0087–0091. This was the baseline design review. P0 is now implemented locally; see [P0 evidence](P0-IMPLEMENTATION.md) for current source and test boundaries.

| Existing module | Preserve | Add or extend |
|---|---|---|
| `crates/voice-agent-server/src/app/mod.rs` | Existing Reminder scheduler start/stop | Exactly one additional Background scheduler with deployment lock |
| `crates/voice-agent-server/src/app/state.rs` | Existing Voice providers/Session state and ActiveDeviceSessions | Reference to BackgroundRuntime; no new session-per-job |
| `crates/voice-agent-server/src/database/notifications.rs` | Reminder domain | Separate `database/jobs.rs`, `database/plugins.rs` |
| `crates/voice-agent-server/src/session/direct_speech.rs` | Direct Speech | No background result speech in V1 |
| `crates/voice-agent-server/src/session/actor/tools/catalog.rs` | Immutable admitted catalog and builtin resolution | Add reserved `server_create_job` definition; independent of `features.mcp` |
| `crates/voice-agent-server/src/session/actor/tools/builtin_actions.rs` | Existing builtin and tool result continuation | Submit asynchronously; completion matched to original turn |
| `crates/voice-agent-server/src/session/actor/tools/round.rs` | Sequential tool calls and `isError` result semantics | Job builtin uses same normal ToolResult, not another WS message |
| `crates/voice-agent-server/src/database/tool_security.rs` | Shared `publication` RwLock | Coordinate worker start with Device/Agent/global/plugin mutation commits |
| `apps/admin-web/src/components/studio/StudioTabs.vue` | Reminders tab | No Background Agent toggle; new Admin pages separate |
| `docs/adr/0087–0091` | Reminder decisions | Job/Plugin ADRs start at 0092, pending fresh numbering check |
| `crates/voice-agent-server/migrations/0104_device_reminders.sql` | Existing schema | New migration uses next free sequence number |

Canonical Background ADRs now live in repository [`docs/adr/`](../adr/). Migration 0105 and P0 source changes preserve ADR 0087–0091 and Reminder migration 0104.

## v1.5.1 wiring-critical code locations

- `app/websocket.rs` constructs the admitted `SessionActor`; provide one shared submit handle through the existing `SessionRuntimes` assembly.
- `session/actor/ingress.rs::run` matches `SessionEvent`; add typed `JobSubmitCompleted` handling and exhaustiveness tests.
- `session/actor/tools/catalog.rs::round_result_delivery` currently falls back to Device MCP policy for non-special tool names; builtin `server_create_job` needs an explicit `LlmThenTts` result.
- `session/actor/tools/round.rs::complete_tool_call` handles `isError`; do not accidentally process Job receipt as MCP direct-response speech.
- Existing Tool Security publication lock is available, but Device and Agent mutation paths must be augmented to participate in worker-start synchronization.
