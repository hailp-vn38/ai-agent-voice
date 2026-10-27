# Vision capability advertises the master Voice bearer token

Status: needs-triage

## Problem

`vision_capability` in
[websocket.rs](../../../crates/voice-agent-server/src/app/transport/websocket.rs) puts
`config.auth.token` (the deployment-wide Voice bearer) into the Device MCP vision capability
sent to every physical Device whose ClientHello advertises `mcp`. Any one Device (or anyone who
reads its MCP traffic) then holds the credential that admits **every** Device identity on the
Voice WebSocket and authorizes the Vision API.

## Why deferred

Fixing it changes the wire contract between server and firmware (what token the Device presents
to `vision.public_url`), so it was explicitly deferred from the WS flow hardening work
(P0–P3, 2026-10-27). No behaviour changed.

## Options to triage

1. A separate `vision.device_token` used only by the Vision API; fall back to `auth.token` while
   unset so existing firmware keeps working.
2. A short-lived, per-Voice-Session vision token minted at admission and revoked with the
   session (strongest; needs Vision API validation against live sessions).
3. Keep as is and document the trust assumption (every Device is fully trusted).

## Acceptance (for option 1 or 2)

- No Device MCP payload contains the Voice bearer token.
- The Vision API rejects the Voice bearer once a dedicated token is configured.
- Firmware compatibility is confirmed on the Reference Device before the fallback is removed.

## Comments
