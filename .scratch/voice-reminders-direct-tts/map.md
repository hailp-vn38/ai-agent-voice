# Voice Reminders and Direct TTS

## Notes

Server/Web implementation first; firmware and hardware acceptance deferred by the user.

## Decisions-so-far

- [01: Shared Direct Speech](issues/01-shared-direct-speech.md): typed origin, shared admitted TTS path, public WS/transcript regressions and design-only golden contracts; full Rust checks pass.

- [02: Admin Direct TTS](issues/02-admin-direct-tts.md): fenced active-session registry and admitted Admin speak route, with a Device Detail test control that separates accepted from played; isolated full Rust validation passes.

- [04: Durable Device Reminder create/list](issues/04-device-reminder-create-list.md): migration0104, atomic replay/quota/revisions, route-bound Admin API, default-deny Agent policy and accessible Device/Agent UI with honest awaiting-sync state.

- [05: Reminder edit/cancel](issues/05-device-reminder-update-cancel.md): scoped rowCAS, atomic schedule revision/idempotent replay, immutable terminal cancellation, edit/conflict/confirmation UI, public ownership/delete/restart regressions.

- [11: Agent Reminder aggregate](issues/11-agent-reminders-tab.md): current-assignment query/KPI/filter/pagination, transaction-bound Agent CRUD scope and keyboard-accessible Reminders tab reusing Device workflows.

## Fog

Firmware offline wake and physical sink calibration remain outside the current server/Web scope.
