# Studio Assistant

Studio Assistant is a browser Voice Protocol Client using the shared `/voice/v1/`
endpoint and the existing Device → Agent → Template → Provider admission pipeline.
The System-owned singleton is a real Device with immutable `kind=web_assistant`;
existing Devices default to `physical`. Browser tabs share the Device, not a Voice
Session. No enrollment, second runtime, Admin MCP tools, or text-only mode is added.

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
Empty capture and admission denial use `rejected`. Capture-only abort may have no
Turn ID. A protocol/transport timeout reports an unknown outcome, never success.
