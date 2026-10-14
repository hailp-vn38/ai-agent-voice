# Voice Reminders / Direct TTS — qualification evidence

Date: 2026-10-12
Integration branch: `integration/voice-reminders-direct-tts`
Implementation baseline: `2092357` (spec, guide, ADR-0087–0091).
Scope: Web/server first, as directed by the user during implementation.
Status: implementation in progress; no release qualification claimed.

## Evidence policy

Only completed commands/gates are recorded as PASS. `tts:stop` is not evidence of sink drain or completed playback. Server protocol clients cannot qualify NVS, I2S/DMA drain, RTC wake, wake-word preservation, connection margin or power-cut recovery.

## Baseline

- Before implementation: Admin Web `npm test -- --reporter=dot`: PASS, 50 files / 211 tests.
- Existing user edits to CONTEXT/GLOSSARY/ADR-0023/0032/0052/0084 and deleted vieneu-native scratch artifacts are preserved.

## Deferred firmware gates

Tickets [12](../.scratch/voice-reminders-direct-tts/issues/12-firmware-online-ack.md), [13](../.scratch/voice-reminders-direct-tts/issues/13-firmware-offline-wake.md), [14](../.scratch/voice-reminders-direct-tts/issues/14-hardware-window-recovery.md) remain open. Real firmware repo/commit, board/audio profile and hardware evidence are NOT_RUN under the Web/server-first scope. Ticket 15 cannot be fully resolved until its required firmware gates pass.

Offline wake cannot be promised for schedules not durably acknowledged by firmware. A lost ACK after playback may cause duplicate delivery under bounded retry. A software sink-drain ACK does not prove sound was physically audible.

## Implemented tickets

- Ticket 01 (`32f84f5`): shared Direct Speech origins and Voice/MCP controls. Format, all-feature Clippy and Rust workspace PASS: 704 passed / 0 failed / 10 ignored across 56 suites. Golden extension fixtures describe contracts; adapter implementation is tracked by subsequent tickets.
- Ticket 04 (`24641b3`): migration 0104, durable Device create/list, Agent grant and Web UI. Format, all-feature Clippy and Rust workspace PASS. Public reminder tests: 3 PASS. Admin Web: 52 suites / 214 tests PASS; final focused tests, typecheck and production build PASS. Live session presence remains unknown until registry integration; cache state remains pending until sync is implemented.

- Ticket 05 (`c83d949`): scoped edit/cancel, row CAS, atomic Schedule Revision, durable replay, UI confirmation/conflict handling. Format, all-feature Clippy and Rust workspace PASS; public reminder tests: 5 PASS; Web: 52 suites / 216 tests PASS; typecheck and production build PASS.

## Final checks

Pending final integration validation and two-axis review. Per-ticket evidence is recorded under each resolved ticket's Answer and linked from the feature map.
