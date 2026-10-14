# Studio Assistant

Studio Assistant is a browser Voice Protocol Client using the shared `/voice/v1/`
endpoint and the existing Device → Agent → Template → Provider admission pipeline.
The System-owned singleton is a real Device with immutable `kind=web_assistant`;
existing Devices default to `physical`. Browser tabs share the Device, not a Voice
Session. It shares the existing voice runtime and ASR/LLM/TTS flow.

All three HTTP routes require the existing Admin bearer and `api.enabled=true`.
Responses use the existing typed Admin error envelope.

| Route | Contract |
| --- | --- |
| `GET /api/admin/system/assistant-device` | 200 settings; an unconfigured deployment returns `configured=false`, nullable identity/assignment/revision, and zero active connections. Never provisions a Device. |
| `PUT /api/admin/system/assistant-device` | `{enabled,agent_key,template_key?}`. First creation needs no revision; subsequent writes require `If-Match: "<revision>"`. Returns 200 settings. A missing/stale update revision or concurrent create returns `409 revision_conflict`. |
| `POST /api/admin/assistant/ws-ticket` | `{}`; requires a trusted `Origin`. Returns 200 `{device_id,client_id,ticket,expires_in_seconds:30}` with `Cache-Control: no-store`. Never creates a Device. |

Settings include `configured`, boolean `enabled`, `device_id`, `agent_key`, nullable
`template_key`, `revision`, and `active_connections`; configured responses also
include `kind`. An override must be an enabled Template with an enabled link to
the selected Agent; null uses the existing default resolution. Settings writes
commit assignment, revision and audit together. Generic Device create/patch
rejects every `kind` field; generic patch/delete of this Device returns
`409 managed_device_use_system_settings`. Manage it from System.

Ticket errors include `401 unauthorized`, `403 untrusted_origin`,
`409 assistant_not_configured/assistant_disabled`, `429 assistant_rate_limited`,
and `503 database_unavailable/shutting_down/assistant_ticket_unavailable`.
Assignment errors use `invalid_agent` and `invalid_template_override`.

## Browser upgrade and revocation

Open `/voice/v1/?protocol-version=1&device-id=<device_id>&client-id=<client_id>&ticket=<ticket>`
using the configured API origin, converting HTTP(S) to WS(S). Do not send either
Admin or Voice bearer on this request. Query duplicates and identity values
provided in both header and query are rejected. A physical/unknown Device cannot
use a ticket; the Virtual Device cannot use the shared physical Voice bearer,
even when physical bearer authentication is disabled.

Tickets contain 256 bits of CSPRNG entropy. Only hashes and minimal bindings live
in RAM. Consume is atomic and irreversible, bound to Device row/identity/revision,
Client ID and Origin, and expires after 30 seconds on a monotonic clock. Failed
upgrade/profile preparation also consumes the ticket. The fixed limits are 20
issues per minute, 32 pending tickets, and two admitted connections (including
handshake). Expired entries are evicted lazily. Origin must match the deployment's
HTTPS public WS origin, or the existing trusted LAN/localhost Origin policy.
Public HTTP origins are not trusted. CORS alone does not protect WS upgrades.

System Save invalidates pending tickets and cancels live/pre-hello web connections.
Publication locks prevent a save between ticket validation and profile admission;
External MCP publication is also locked during save/revocation so new tool dispatch
cannot cross that boundary. Runtime shutdown continues through the existing drain.
Browser token changes/app teardown disconnect and clear the local conversation;
a local logout cannot revoke another tab's shared Admin token. Restart discards
all tickets. Already-dispatched tool side effects cannot be undone or replayed.

Never log tickets, raw upgrade queries, or raw auth frames. App tracing records
URI paths. The supplied Nginx config logs paths only and suppresses WS-location
error logs because Nginx error messages include the full request URL. Apply the
same policy at any outer proxy/CDN/error reporter.

## Tool execution status (mặc định cho Studio Assistant)

Mọi Virtual Device phiên Studio Assistant được server tự bật tool status sau
xác thực, ngay cả khi ClientHello không khai báo `features.tool_status`.
Compact Voice hiện vẫn gửi `features.tool_status=true` và giữ
`features.mcp=false`, `features.turn_status=true`. `mcp:false` **chỉ tắt
Device MCP**; Builtin Tool và Agent-scoped External MCP chạy phía server và
vẫn phát event `type:"tool"` khi LLM thực sự gọi tool. `tool_status` nằm trong
ClientHello (client → server), không phải field server echo ở ServerHello.
Không thêm quyền tool mới và không lộ arguments/results.

```json
{"type":"tool","session_id":"...","turn_id":1,"name":"external.home_assistant.light_turn_on","state":"started"}
```

`state` là `started` / `completed` / `failed`. Message không chứa
`call_id`, `source`, `reason`, arguments hoặc results. Floating Chat chỉ
hiển thị tên tool đang chạy dưới waveform khi đang processing, dùng state
`activeTool` tạm thời và xóa khi tool terminal, abort, turn terminal hoặc
mất kết nối. Không thêm status vào lịch sử ASR/LLM. Do control queue bounded,
events là best-effort; terminal turn status vẫn quyết định lúc mở lượt mới.

## Opt-in turn status

Only authenticated Virtual Device sessions with `features.turn_status=true`
receive `turn_status`. Other clients keep their existing wire events. Web sessions
never enable Device MCP; Agent-scoped External MCP continues through its existing
allowlist, revision/fingerprint and sensitive-tool guards.

```json
{"type":"turn_status","session_id":"...","state":"listening_ready"}
{"type":"turn_status","session_id":"...","state":"rejected","reason":"asr_capacity"}
{"type":"turn_status","session_id":"...","state":"completed","turn_id":1,"outcome":"normal"}
```

`listening_ready` follows the manual ASR worker's Opened event. Wait for it before
sending `listen:detect` or microphone Opus. `stt` is the accepted-user echo.
`llm` segments belong to one response; raw binary downlink remains 24 kHz mono
60 ms Opus. Terminal states are `completed`, `failed`, and `cancelled`; a Turn ID
is included once one exists. Normal completion follows the writer's terminal
outcome after `tts:stop`; early LLM/TTS/recognition failures also release the UI.
For normal completion, Floating Chat waits for the decoder flush and all queued
audio to finish before accepting another turn. Playback buffers 1.28 seconds at
startup and after starvation, bounded to two seconds; a short final tail plays
after the flush without waiting for the buffer threshold. An 8 ms gain fade
smooths playback starts and ends. Auto microphone uplink pauses during the final
local drain and resumes after it; Interrupt can stop that drain immediately.
Empty capture and admission denial use `rejected`. Capture-only abort may have no
Turn ID. A protocol/transport timeout reports an unknown outcome, never success.

## Compact Voice + Live Conversation web client

The AdminShell mounts Studio Assistant once across the web app. The floating panel is
voice-first (375 px wide, up to 490 px high): compact status dot, microphone and
waveform, plus a Live Conversation area that shows the last user/ASR message paired
with the streaming LLM response. The History toggle reveals earlier messages;
the keyboard button opens an optional text composer. ASR is displayed when the
server emits an accepted `stt` event; no speculative partial transcription is
fabricated. `llm` segments are appended to the active response until terminal
`turn_status`. Playback status is shown only when the speaker is enabled.

Clicking anywhere outside the panel (or pressing Escape) collapses the UI into a
persistent bubble without closing the WebSocket, cancelling a turn or stopping TTS.
Clicking the bubble reopens the same client and in-memory conversation. The header
X deliberately ends the session (confirming first if a turn is active); logout
and layout teardown also disconnect. A backgrounded browser tab still interrupts
active microphone capture, while TTS downlink remains unaffected by panel collapse.
Manual microphone capture is interrupted on panel collapse to avoid hidden recording.
The default microphone action unlocks TTS playback within the user gesture, unless
the user explicitly muted the speaker. Statuses remain separate: a completed
WebSocket handshake does not guarantee success of the current Agent turn.

## Read-only System MCP

`[system_mcp] enabled = false` is the default deployment ceiling. Set `enabled = true`
and reconnect Studio Assistant to publish six read tools: `system_get_status`,
`system_get_resources`, `mcp_list_servers`, `mcp_get_server`, `mcp_list_tools`, and
`mcp_get_agent_bindings`. Only an origin-bound ticket admitted to the managed Virtual
Device receives `system:read` and `mcp:read`. Physical Devices, including ones assigned
to the same Agent, never receive these tools. `features.mcp=false` continues to disable
Device MCP only; `tool_status=true` reports existing `tool` progress messages.

System MCP calls run sequentially within the existing turn budget and cancellation
boundary. The catalog is fixed at admission; dispatch rechecks the enabled Device and
Agent and revocation. Results use the existing bounded Tool Result path and continue
the LLM/TTS response. MCP metadata excludes endpoints, headers, credentials, prompts,
raw errors, and tool schemas/descriptions. Reading observed tools never approves or
executes External MCP tools.

`mcp_list_servers` accepts `page` (1–10000) and `page_size` (1–100, default 20).
`mcp_list_tools` accepts optional `agent_key` (defaults to the admitted Agent),
`server_key`, and `page`; pages contain at most 20 observed tools, including stale
contracts marked `current=false`. `allowed` describes current contract approval;
`sensitive` is separate, and these fields do not assert that a tool is callable.
`mcp_get_agent_bindings` returns at most 100 bindings with both binding/server enabled
states, or a bounded error. All outputs are subject to `llm.max_tool_result_chars`;
oversized results fail with `tool_result_too_large`, and server lists can be requested
with a smaller page. An unavailable/stale OS resource snapshot returns a typed error.

There are no System MCP HTTP routes, discovery, grant settings, or mutations in P1.
