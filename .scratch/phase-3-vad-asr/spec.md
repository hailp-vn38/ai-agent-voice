# Phase 3 — VAD + ASR local streaming

Status: ready-for-agent

## Problem Statement

Voice-agent server hiện chỉ thu Manual Capture rồi discard audio. Người dùng chưa thể nói vào Voice Protocol Client để server tự xác định utterance, chạy ASR local, giữ context hội thoại trong RAM và trả STT tương thích firmware. Hệ thống cũng chưa có boundary an toàn cho local inference: state streaming, cancellation, capacity, worker health và overload có thể làm sai ranh giới utterance hoặc tạo transcript không đáng tin.

## Solution

Triển khai Phase 3 với Silero VAD local và Zipformer streaming ASR local sau ProviderSet được load/warmup trước bind socket. SessionActor chỉ điều phối state domain qua bounded, pinned VAD/ASR worker pools; provider runtime không biết WebSocket hay actor state. Auto và Manual cùng tạo ASR final theo lifecycle riêng, commit user message vào Dialogue History RAM-only, rồi gửi tối đa một STT V1 hiện có trước khi terminal theo listen mode.

## User Stories

1. As a Voice Protocol Client user, I want Manual capture to produce a final STT result, so that I can verify speech recognition without VAD.
2. As a Voice Protocol Client user, I want Auto capture to recognize a spoken utterance without manually stopping it, so that interaction feels hands-free.
3. As a Voice Protocol Client user, I want trailing silence to end Auto capture predictably, so that one sentence does not become multiple turns.
4. As a Voice Protocol Client user, I want pre-roll retained at SpeechStart, so that the first phoneme is not lost.
5. As a firmware-compatible client, I want exactly one existing `stt` message for a valid non-empty final, so that no new wire protocol is required.
6. As a firmware-compatible client, I want no partial-ASR or VAD wire messages, so that V1 compatibility remains intact.
7. As a user, I want empty recognition to complete silently, so that silence never starts a false dialogue turn.
8. As a user, I want abort or replacement listen:start to cancel old work, so that stale STT is never shown after a newer interaction begins.
9. As a user, I want audio dropped while Processing, so that a second utterance cannot race the first finalization.
10. As an operator, I want providers/model artifacts validated and warmed before bind, so that a broken local model never accepts public traffic.
11. As an operator, I want ASR stream capacity bounded globally, so that a few sessions cannot oversubscribe local CPU.
12. As an operator, I want VAD capacity bounded globally, so that each Auto cycle has isolated recurrent state without unbounded threads.
13. As an operator, I want overloaded ASR to fail the recognition rather than omit audio, so that no incomplete transcript is presented as valid.
14. As an operator, I want VAD queue pressure to be privacy-safe and observable, so that transient latency does not close healthy sessions.
15. As an operator, I want unhealthy workers quarantined, so that a stuck native runtime is never silently reused.
16. As an operator, I want one damaged worker to affect only its Voice Session, so that healthy sessions and server startup remain available.
17. As a future Phase 4 implementer, I want user text committed to bounded Dialogue History before STT, so that LLM/TTS can extend the same turn boundary without rewriting ASR semantics.
18. As a privacy-conscious user, I want audio and transcript excluded from telemetry, so that operational metrics do not leak conversation content.
19. As a reference-client maintainer, I want `docs/audio.wav` exercised through canonical Opus transport, so that local VAD/ASR smoke validation reflects client behavior.
20. As a test author, I want deterministic fake providers injected at the application boundary, so that protocol and session tests do not require large models or CPU inference.

## Implementation Decisions

- Production loads config, ProviderSet, model artifacts and warmup before creating the router or binding a socket. Router/session receive injected runtime dependencies; tests inject deterministic fake providers through the same public boundary.
- Default providers are local `silero_onnx` VAD and `zipformer_sherpa` streaming ASR. Python sidecars and HTTP ASR are outside the Phase 3 baseline.
- VAD returns model-level probability/frame data only. Core VadSegmenter owns hysteresis, pre-roll, minimum speech duration, trailing silence and max utterance semantics.
- Auto acquires and pins a VadSession to a bounded VAD worker for its complete Auto Listening cycle. Manual never acquires a VAD worker.
- VAD Reset is an acknowledgement barrier between Auto utterances. Re-arm occurs only after ResetDone; Close releases a VAD slot only after Closed acknowledgement.
- SpeechStart in Auto, and listen:start in Manual, acquire an AsrStreamLease and open a streaming ASR session pinned to one bounded ASR worker until terminal cleanup.
- ASR worker pool is not a generic job queue. Open, Push, Finish and Cancel for one stream remain on its pinned worker; worker events carry session identity, generation and stream identity.
- ASR partial hypotheses remain inside provider/worker. Phase 3 does not read, store, log, emit or use them to reset a timeout.
- At SpeechEnd or listen:stop, actor enters Processing, tries Active Turn Limiter, then starts ASR finalization. The limiter belongs to application runtime, not ProviderSet.
- Phase 3 terminal after current-generation non-empty AsrFinal is: commit user to Dialogue History, enqueue exactly one existing STT payload, release resources, then Manual returns Ready and Auto resets/re-arms Listening.
- Empty final, failure, timeout, stale final, denied Active Turn permit and ASR overload send no STT and commit no user message.
- Generation advances before every replacement/cancellation boundary. Natural SpeechEnd and listen:stop retain the current generation so their final can be accepted.
- Cancellation invalidates logical events immediately, but ASR/VAD worker slots release only after worker cleanup acknowledgement. Cleanup timeout quarantines the worker; ASR failure fail-closes the affected session when ownership is unsafe.
- VAD inference, Reset or Close failure/timeout is a session-scoped fatal condition: invalidate generation, cancel dependent ASR, quarantine VAD worker and close affected WebSocket with 1011. Never silently fall back from Auto to Manual.
- VAD queue full drops only the VAD input frame and records privacy-safe metadata while preserving input sample timeline. ASR queue full cancels recognition; Auto ignores ASR until the current SpeechEnd to avoid fragmenting one utterance.
- `asr.timeout_ms` starts at endpoint and bounds Finish until terminal result. Streaming Push is bounded by max utterance, queue capacity, cancellation and runtime errors rather than a lifetime timeout.
- Dialogue History is RAM-only per Voice Session, bounded by `llm.max_history_messages` with default 20. Actor uses only commit_user; history owns eviction so Phase 4 can evolve to Exchange Atom eviction.

## Module Layout Decision

Phase 3 tách adapter, worker ownership và SessionActor thành module Rust riêng thay vì tiếp tục mở rộng một provider file đồng bộ. Layout đích được dùng khi triển khai Phase 3 là:

```text
crates/voice-agent-server/src/
├── app.rs
├── config.rs
├── lib.rs
├── audio/{mod.rs, pcm.rs, opus.rs, resample.rs, frame_buffer.rs, vad_segmenter.rs, pacer.rs}
├── providers/
│   ├── {mod.rs, error.rs, registry.rs, set.rs, capabilities.rs}
│   ├── vad/{mod.rs, traits.rs, silero_onnx.rs}
│   ├── asr/{mod.rs, traits.rs, zipformer_sherpa.rs}
│   └── tts/{mod.rs, traits.rs, zerotts_onnx/...}
├── workers/{mod.rs, vad.rs, asr.rs, tts.rs}
├── session/{actor.rs, event.rs, state.rs, turn.rs}
└── speech_output/{mod.rs, command.rs, worker.rs}
```

Trong Phase 3, chỉ materialize các module cần cho VAD/ASR: `audio::vad_segmenter`, provider core/VAD/ASR, `workers::vad`, `workers::asr`, `session::event` và `session::turn`. `providers::tts`, `workers::tts` và `speech_output` được reserve cho Phase 4; không tạo adapter TTS rỗng hoặc kéo dependency TTS vào Phase 3. Worker runtime thuộc application, provider adapter không biết worker/session/WebSocket, còn SessionActor chỉ gửi command và nhận typed event.

## Testing Decisions

- Prefer the application/router and SessionActor event boundary over worker internals. Tests observe phase, STT payload/order, terminal mode, resource-release outcome and close code rather than recognizer private state.
- Contract tests run against fake VAD/ASR providers at the injected ProviderSet seam: open/push/finish/cancel, current-generation final exactly once, empty final, timeout, cancellation and bounded capacity.
- Session tests cover Manual and Auto: Processing drops binary audio, Auto opens ASR only after SpeechStart, pre-roll feeds first audio, SpeechEnd finalizes the same generation, and terminal mode differs by listening mode.
- Worker tests cover stream pinning, acknowledgement-driven slot release, ResetDone re-arm barrier, quarantine after cleanup timeout, VAD queue drop and ASR queue terminal failure.
- Dialogue tests cover bounded `commit_user`, oldest eviction through history API, non-empty-final ordering, and no commit for empty/failure/stale outcomes.
- WebSocket integration tests use fake providers to assert exactly one existing STT message before terminal state and absence of partial/VAD wire messages.
- Reference Client smoke tests encode `docs/audio.wav` to canonical 16 kHz Opus 60 ms and execute both Manual and Auto scenarios against a real-model configuration; live smoke is separate from deterministic CI.
- Existing prior art includes protocol state tests, Opus/manual-capture tests, reference-client WAV tests, VAD/ASR flow documentation and E2E test structure.

## Out of Scope

- LLM, MCP, TTS, SpeechOutput and assistant delivery; Phase 3 ends after STT terminal behavior.
- Acoustic barge-in, AEC, VAD while Speaking, new WebSocket event types and protocol capability negotiation.
- Dynamic provider plugins, model hot swapping, Python sidecars, HTTP ASR baseline, automatic provider retries and model downloads during a Voice Session.
- Persistent memory, cross-session Dialogue History, transcript/audio logging, voice cloning and non-V1 audio profiles.
- Automatic replacement of quarantined workers and server-wide shutdown due to one runtime worker failure.
- TTS provider, TTS worker và SpeechOutput implementation; các nhánh tương ứng trong layout chỉ là reserved boundary cho Phase 4.

## Further Notes

- Follow ADR-0017, ADR-0022, ADR-0039, ADR-0040 and ADR-0041.
- Zipformer model artifacts are local deployment inputs and must not be committed to normal Git history; its license remains a deployment gate.
- `docs/audio.wav` local smoke confirms non-empty VAD/ASR behavior but is not a substitute for the real Voice Protocol Client interoperability gate.
