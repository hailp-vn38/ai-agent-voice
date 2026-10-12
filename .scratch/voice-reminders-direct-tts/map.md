# Voice Reminders and Direct TTS

## Notes

Server/Web implementation first; firmware and hardware acceptance deferred by the user.

## Decisions-so-far

- [01: Shared Direct Speech](issues/01-shared-direct-speech.md): typed origin, shared admitted TTS path, public WS/transcript regressions and design-only golden contracts; full Rust checks pass.

- [04: Durable Device Reminder create/list](issues/04-device-reminder-create-list.md): migration0104, atomic replay/quota/revisions, route-bound Admin API, default-deny Agent policy and accessible Device/Agent UI with honest awaiting-sync state.

## Fog

Firmware offline wake and physical sink calibration remain outside the current server/Web scope.
