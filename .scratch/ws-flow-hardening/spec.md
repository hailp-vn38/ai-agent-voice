# WS flow hardening

Status: ready-for-agent

Origin: review of the Voice Session WebSocket flow (2026-10-27). Decisions taken with the user
are recorded here; implementation commits reference this directory.

## Scope

| Group | Item | Decision |
| --- | --- | --- |
| P0 | Whole-message transport cap (`max_message_size`) | Done |
| P0 | Physical Devices never wait on Web Assistant credential publication | Done |
| P1 | Transport idle, write timeout, Ping before ClientHello | Per ADR 0020: idle 300 s on RX **and** TX, no server heartbeat by default (Pings opt-in), write timeout 10 s |
| P1 | Every turn-scoped writer command ends in exactly one terminal outcome | Done; writer failures report `WriterEvent::Failed` once |
| P2 | Voice Session admission module (`admit` → `Admission` / `Rejection`), every refusal logged | Done |
| P2 | WebSocket Writer module with I/O-free playback arbitration | Done |
| P3 | Collapse external close signals (`SessionCloseSet`) | Done |
| P3 | `SessionActor::start` replacing the builder chain | Not pursued: a parameter struct over the same ~20 settings fails the deletion test (pass-through); revisit only if a second caller appears |
| P3 | Reminder protocol out of transport | Done (`ReminderCoordinator::on_device_reminder_event`) |
| P3 | Ticket not burned by a capacity refusal | Done (429 keeps the ticket; other refusals burn it) |
| P3 | Constant-time bearer comparison; query credential guidance | Done |
| P3 | Vision capability exposes the Voice bearer | Deferred: [issue 01](issues/01-vision-device-token.md) |

## Test seams agreed

Integration at the real WebSocket only (`tests/ws_protocol.rs`, `tests/web_assistant.rs`).
