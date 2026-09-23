# Phase 5 — Interruption và acoustic barge-in correctness

Status: ready-for-agent

## Problem Statement

Voice Session hiện có cancellation theo generation, nhưng invalidation outbound còn phụ thuộc queue control bounded và chỉ bảo vệ binary audio. Khi assistant đang `Speaking`, microphone bị drop nên Voice Protocol Client có uplink đã echo-suppressed không thể nói chen. `listen:start` hiện còn lẫn nghĩa arm capture với interrupt, `Realtime` chưa chạy runtime, và lifetime VAD worker bị lẫn với semantic capture cycle. Các gap này có thể gây stale `llm`/`tts:start`/audio, mất prefix utterance gây interruption, hoặc nhận event VAD cũ như event của turn mới.

## Solution

Phase 5 đưa interruption về một contract duy nhất thuộc SessionActor: mỗi Conversational Turn có GenerationId và CancellationToken riêng; shared GenerationGate chặn mọi turn payload tại WS writer; urgent lane bảo đảm `tts:stop` hoặc fail-closed. Acoustic Barge-in là opt-in client-AEC cho Auto/Realtime: VAD xác nhận SpeechStart, actor snapshot PCM retention trước reset, invalidate/cancel turn cũ, phát đúng một stop khi playback đã bắt đầu, rồi mở ASR cho turn mới trong cùng Voice Session.

Server không làm AEC. `features.aec=true` chỉ là Echo-safe Client Assertion và chỉ có hiệu lực khi deployment bật rõ hai config Barge-in. Manual không acoustic barge-in.

## User Stories

1. As a Voice Protocol Client user, I want explicit `abort` to stop the current assistant response, so that I can end an unwanted response immediately.
2. As a Voice Protocol Client user with echo-suppressed uplink, I want to speak while the assistant is Speaking, so that a new utterance can interrupt the old response naturally.
3. As a Voice Protocol Client user, I want the first PCM that triggered barge-in retained, so that the beginning of my new utterance is never lost.
4. As a Voice Protocol Client user, I want the same Voice Session to continue after barge-in, so that I do not need to reconnect before the next response.
5. As a Manual Listening Mode user, I want microphone audio during Speaking not to auto-interrupt, so that playback echo cannot create a false turn.
6. As a client without trusted AEC capability, I want the existing non-barge-in behavior retained, so that rollout does not unexpectedly cut off assistant playback.
7. As an Auto Listening Mode user, I want barge-in only after a capture cycle is armed, so that inactive capture does not interpret arbitrary microphone data as an interruption.
8. As a Realtime Listening Mode user, I want VAD to remain armed through Processing and Speaking, so that I can barge in without a second `listen:start`.
9. As a Voice Protocol Client user, I want `listen:start` to arm or reset capture rather than cancel a response, so that capture control and interruption control are unambiguous.
10. As a Voice Protocol Client user, I want `tts:stop` exactly once after a started response is interrupted, so that my playback state remains coherent.
11. As a Voice Protocol Client user, I want no stale `llm`, `tts:start`, normal `tts:stop`, or Opus payload after interruption, so that an old answer cannot revive.
12. As a Voice Protocol Client user, I accept that a frame already sent before the interruption boundary cannot be recalled, so that the delivery guarantee is technically honest.
13. As an operator, I want client AEC assertion disabled and untrusted by default, so that existing clients retain safe behavior until I opt in.
14. As an operator, I want a bounded urgent control lane, so that cancellation control is not starved by normal control or audio backlog.
15. As an operator, I want an urgent stop admission failure to fail closed, so that the server never continues with an unknown client playback state.
16. As an operator, I want bounded PCM retention derived from VAD confirmation and queue lag, so that barge-in is prefix-safe without unbounded memory.
17. As an operator, I want VAD Worker Lease and VAD Capture Cycle to have separate identity, so that delayed semantic VAD events cannot start an incorrect turn.
18. As an operator, I want cleanup acknowledgement still consumed after a turn or capture cycle is stale, so that native worker capacity is never reused prematurely.
19. As a dialogue user, I want an interrupted Generated Assistant Response excluded from Delivered Assistant Response history, so that future context reflects only audio that drained.
20. As an implementation maintainer, I want one interruption primitive, so that abort, Acoustic Barge-in, and session failure do not drift into conflicting cancellation orders.
21. As an implementation maintainer, I want turn payload tagging at the outbound boundary, so that producer cancellation is an optimization rather than the only stale-output defense.
22. As a test maintainer, I want deterministic tests to observe protocol-visible ordering through the application boundary, so that queue scheduling and stale filtering are proven without coupling to private internals.
23. As a Reference Client maintainer, I want an end-to-end two-utterance scenario with a post-stop quiet period, so that stale old audio is observable independently of unit tests.
24. As a release owner, I want real ZeroTTS plus Reference Client validation kept distinct from fake-provider tests, so that Phase Completion evidence reflects the real delivery path.
25. As a release owner, I want physical firmware playback reported as HIL evidence rather than inferred from CI-compatible clients, so that hardware scope remains honest.
26. As a future server-AEC implementer, I want client-AEC Barge-in not to imply server AEC, so that timestamp/reference alignment can be designed as a separate protocol boundary.

## Implementation Decisions

- ADR-0045 is authoritative and supersedes the prior no-acoustic-barge-in decision. It preserves Single WebSocket Writer, SpeechOutput ownership, cleanup acknowledgement, Dialogue History delivery-commit, and bounded-worker contracts.
- Add optional ClientHello capability `features.aec`, defaulting to false and preserving unknown-feature compatibility. It is an Echo-safe Client Assertion, not evidence of server-side AEC.
- Acoustic Barge-in predicate requires all of: `barge_in.enabled`, `barge_in.trust_client_aec_feature`, client `aec=true`, and Auto or Realtime Listening Mode. Both deployment config values default false.
- `listen:start` sets or replaces Listening Mode and arms/resets a VAD Capture Cycle without cancelling a Conversational Turn or changing GenerationId. Explicit `abort` always interrupts. Manual never acoustic-interrupts.
- Auto and Realtime reuse one VAD/ASR/retention path. Auto only watches during Speaking when an appropriate capture cycle is armed; Realtime retains its capture cycle through Processing and Speaking.
- Separate GenerationId, VAD Worker Lease identity, and VAD Capture Cycle identity. Semantic VAD events require the current cycle identity; stale cleanup acknowledgement remains actionable.
- Each Conversational Turn owns a fresh CancellationToken derived from a session root token. A centralized interruption primitive orders: snapshot if needed; invalidate shared GenerationGate; cancel turn producers; request worker cleanup; release Active Turn capacity; urgent-stop a previously Started response; establish the next turn identity.
- GenerationGate is shared by actor and writer and is the linearization boundary. Every turn-scoped JSON and audio payload is checked immediately before writer admission/send. A frame already admitted before invalidation cannot be recalled.
- Writer scheduling is bounded `urgent > normal control > audio`. Urgent lane is limited to interruption `tts:stop`, close and fatal/session control. Failure to admit interruption stop after playback Started is a Voice Session integrity failure and uses root cancellation/writer shutdown fail-closed escape path.
- Reuse and generalize AutoPcmRetention for Auto, Realtime and Barge-in. It overwrites oldest PCM and sizes capacity as pre-roll plus VAD confirmation horizon plus bounded VAD command lag plus one uplink frame plus rechunk slack. Current defaults derive 39,872 samples at 16 kHz; no fixed 500 ms ring is introduced.
- On allowed Barge-in SpeechStart, snapshot retained PCM from onset minus pre-roll through current cursor before reset. Then invalidate old turn N, cancel it, urgent-stop if needed, create N+1, open ASR N+1, feed snapshot, and route subsequent PCM to the new ASR stream.
- SpeechOutput remains generation-aware, resets pacing/buffer/resampler state on cancel, and cannot relabel old output as a new turn. Interrupted assistant content is not committed as Delivered Assistant Response.
- Phase work is split into six dependency-ordered tickets: outbound gate/scheduling; turn cancellation primitive; Speaking/AEC protocol/config; VAD Capture Cycle and Realtime; Barge-in integration; qualification gates.

## Testing Decisions

- The primary seam is the existing public application/router boundary exercised by the Rust Reference Client or a deterministic Voice Protocol Client. Tests assert visible control/audio ordering, Voice Session continuity, accepted STT, dialogue commit, and terminal behavior; they do not inspect writer queues, ONNX state, or mutable worker internals.
- Existing SessionActor, WS writer, VAD/ASR worker-runtime, SpeechOutput, and Reference Client test seams are extended rather than introducing a parallel test transport.
- Protocol tests cover missing/false/true `features.aec` and ignored unknown feature fields.
- Deterministic application tests cover explicit abort, no-AEC behavior, Manual behavior, unarmed Auto behavior, Realtime behavior, exactly-one interruption, normal completion, and no stale JSON/audio after gate invalidation.
- VAD/ASR tests cover retention snapshot range, triggering-frame preservation, calculated retention bound, late VAD Capture Cycle event rejection, and processing of stale cleanup acknowledgement.
- Writer tests fill normal control/audio queues before interruption and prove `urgent > normal control > audio`, no stale turn payload admission after the gate boundary, and fail-closed behavior if urgent stop cannot be admitted.
- Lifecycle tests cover late LLM/TTS events, old Drained event, double interruption, Active Turn release exactly once, session disconnect during Barge-in, and worker quarantine/cleanup acknowledgement rules.
- Reference Client E2E uses utterance A to reach started TTS, sends utterance B during playback using trusted AEC Auto/Realtime, verifies exactly one stop for A, verifies B reaches STT then LLM/TTS on the same socket, and observes a quiet period after stop with no stale A payload.
- The mandatory completion gate combines deterministic test evidence with the existing real ZeroTTS and Reference Client delivery path. Firmware playback, real AEC effectiveness, and physical device behavior are separate HIL gates.

## Out of Scope

- Server-side AEC, including timestamp/reference-audio protocol fields, drift handling, reference cache, or an EchoCanceller provider boundary.
- Acoustic Barge-in for Manual Listening Mode.
- Barge-in during Processing before assistant audio has Started.
- New V1 WebSocket message types for VAD, ASR partials, timestamps, or AEC diagnostics.
- Automatic retry of ASR, LLM, TTS, VAD, or interruption operations.
- Persistent dialogue, transcript/PCM logging, dynamic providers, model acquisition at runtime, Device MCP Phase 6 work, and voice cloning.
- Treating fake-only tests, a build, or an optional live-provider smoke as Phase 5 completion evidence.

## Further Notes

- Phase 3 Auto VAD reset acknowledgement and Phase 4 TTS cleanup acknowledgement remain prerequisites; Acoustic Barge-in must not bypass their quarantine/reuse semantics.
- The Reference Client E2E proves public protocol behavior but does not prove physical microphone, client AEC quality, or speaker playback. Those must be reported separately as HIL.
- Each ticket should stage only scoped changes and claim completion only after its own exit gate passes. The phase is complete only after the final qualification ticket has the required deterministic and real-model evidence.
