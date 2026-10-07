# Speaker Observe — ESP32 Voice WS smoke evidence (ticket 10)

Scope: Agent policy `observe` scoring a real ESP32 uplink over the production Voice WS.
This is **diagnostic evidence only** — it makes no accuracy claim and does not qualify
`Required` (that is ticket 15).

## Result

| Field | Value |
| --- | --- |
| Status | **NOT_RUN** |
| Reason | No ESP32 device attached to this build host. |
| Date | 2026-10-08 |
| Build | `speaker/10` |
| Firmware | n/a |

`NOT_RUN` is an accepted outcome for this ticket. Do not record a PASS without a device.

## How to run (operator, with hardware)

1. Configure the Agent policy to `observe` and grant a finalized, enrolled speaker on the
   Agent's active Template.
2. Point the ESP32 at the server Voice WS and speak a phrase that should match the speaker.
3. Observe the separate `speaker` status field on the WS — it must carry a bounded state
   (e.g. `verified` / `insufficient`) plus queue/quality/inference timing, and **must not**
   carry speaker identity, key or raw score.
4. Confirm the ASR/barrier status is reported separately from the speaker status.
5. Confirm `off` produces no speaker frame and no extra PCM work.

## What is already covered without hardware

`crates/voice-agent-server/tests/speaker_observe.rs` deterministically exercises the same
actor path with synthetic PCM:

- `observe_off_emits_no_speaker_frame_and_retains_nothing`
- `observe_emits_bounded_state_without_identity_or_score`
- `observe_unavailable_runtime_does_not_block_core_path`

These prove the gate, the bounded payload and the non-blocking behaviour. The hardware smoke
adds only end-to-end device timing, which is why its absence is recorded honestly rather than
papered over.
