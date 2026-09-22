# Phase 4 — OpenAI LLM streaming va ZeroTTS delivery

Status: completed; compatibility follow-up 09 is ready-for-agent

## Problem Statement

Sau ASR final hop le, Voice Protocol Client hien chi nhan `stt` va Conversational Turn ket thuc. Nguoi dung chua nhan duoc Generated Assistant Response bang audio, Dialogue History chua duoc dung de tao phan hoi, va server chua co boundary an toan cho remote LLM streaming hoac local ZeroTTS inference. Khong co ownership ro rang cho sentence delivery, pacing, cancellation, capacity, warmup va audio ordering se de stale audio, `tts:start`/`tts:stop` sai thu tu, provider overload khong determininistic, hoac mot Model Artifact pack loi van duoc server public bind.

## Solution

Mo rong Provider Registry compile-time va startup loader de tao VAD, ASR, LLM va TTS truoc public bind. LLM dau tien la typed `openai` adapter dung crate Rust `llm` pin exact; ZeroTTS dau tien la `zerotts_onnx` local native Rust/ONNX, duoc Model Preparation resolve va deterministic warmup.

Sau AsrFinal non-empty hien tai, SessionActor dieu phoi LLM Operation theo generation. Text delta qua Sentence Segmenter thanh Speech Segment bounded, duoc SpeechOutput giao cho TtsWorkerRuntime. SpeechOutput so huu synthesis ordering, PCM normalize/resample, Opus 24 kHz, AudioPacer va `FinishInput`/`Drained`; SessionActor van la outbound producer duy nhat. Normal delivery chi commit Delivered Assistant Response sau Drained. Cancellation, failure va overload invalidate GenerationGate truoc khi stale audio co the den Voice Protocol Client.

## User Stories

1. As a Voice Protocol Client user, I want a non-empty ASR final to start an assistant response, so that a spoken request receives a spoken answer.
2. As a Voice Protocol Client user, I want the first audio of a long assistant response before LLM streaming finishes, so that the interaction feels responsive.
3. As a Voice Protocol Client user, I want `tts:start` before the first binary audio packet, so that playback state begins only when playable audio exists.
4. As a Voice Protocol Client user, I want exactly one `tts:stop` after normal delivery drains, so that playback ends predictably.
5. As a Voice Protocol Client user, I want no `tts:start` or `tts:stop` when failure happens before audio starts, so that the protocol never emits a fake playback lifecycle.
6. As a Voice Protocol Client user, I want no audio for an invalidated generation after its stop control, so that aborted or failed replies do not continue speaking.
7. As a Voice Session user, I want a newer cancellation or replacement to stop remaining LLM and TTS work, so that an old Conversational Turn cannot revive the current interaction.
8. As a Voice Session user, I want only fully drained assistant audio committed to Dialogue History, so that future context reflects what I actually heard.
9. As a user, I want an LLM timeout or remote provider failure to fail only my current generation, so that a transient remote outage does not make the server globally unavailable.
10. As a privacy-conscious user, I want prompts, responses, audio, API keys and provider bodies excluded from logs and telemetry, so that conversations and credentials are not exposed.
11. As an operator, I want OpenAI configuration validated locally at startup without a network probe, so that startup is deterministic and remote availability is evaluated per LLM Operation.
12. As an operator, I want the OpenAI provider built through a pinned Rust crate API, so that the server does not own vendor HTTP/SSE parsing or silently change dependency behavior.
13. As an operator, I want a fixed compiled Provider Registry, so that selection is typed and no plugin, hot-load or runtime code discovery can change production behavior.
14. As an operator, I want ZeroTTS Model Artifacts resolved, verified and warmed before bind, so that an incomplete local model pack never accepts traffic.
15. As an operator, I want ZeroTTS warmup to exercise tokenizer, voice, TTS graphs and codec using non-user content, so that startup detects runtime incompatibility before a Voice Session exists.
16. As an operator, I want the named `maichi` voice and composite model license acknowledged exactly, so that the active voice latent and bundled-codec license are auditable.
17. As an operator, I want all ZeroTTS PCM normalized to an explicit 48 kHz mono provider boundary before conversion to the Canonical Audio Profile, so that audio conversion never depends on guessed vendor output.
18. As an operator, I want LLM concurrency globally bounded by LlmRuntime, so that remote streaming cannot create unbounded tasks.
19. As an operator, I want ZeroTTS mutable inference owned by bounded native workers, so that it never blocks the Tokio executor and damaged workers can be quarantined.
20. As an operator, I want TTS admission capacity equal native worker capacity, so that a granted turn cannot be denied by a second mismatched limiter.
21. As an operator, I want a bounded pending Speech Segment queue, so that a fast LLM cannot grow memory while TTS is slower than generation.
22. As a user, I want sentence order preserved even when synthesis and playback overlap, so that the assistant never speaks a later sentence before an earlier one.
23. As an operator, I want a TTS segment timeout that does not reset for intermittent PCM chunks, so that a stalled synthesis cannot retain a worker indefinitely.
24. As an operator, I want cleanup acknowledgement before a cancelled TTS worker slot is reused, so that stale native inference state cannot leak into another generation.
25. As an operator, I want queue overflow, missing worker capacity and unexpected tool calls to fail the generation instead of dropping text or audio silently, so that delivery semantics remain honest.
26. As a future Phase 6 implementer, I want a typed LLM stream seam that supports tools but receives no tool definitions in Phase 4, so that tool-result orchestration can be added without changing SessionActor ownership.
27. As a Reference Client maintainer, I want real ZeroTTS output encoded as valid canonical downlink Opus, so that compatibility testing exercises the same delivery boundary as an independent client.
28. As a release owner, I want live OpenAI validation separated from the mandatory local Phase Completion Gate, so that CI is deterministic and does not require network, API quota or paid credentials.

## Implementation Decisions

- Extend the fixed Provider Registry and ProviderSet with LlmFactory/LlmProvider and TtsFactory/TtsProvider alongside VAD/ASR. Startup builds all four selected providers before socket bind; no dynamic plugin, `dlopen`, hot-load or runtime discovery is allowed.
- Configure LLM as typed `providers.llm.type = "openai"`. Pin `llm` to `=1.3.8`, disable default features, and enable only `openai` plus Rustls TLS. The adapter maps typed configuration into `LLMBackend::OpenAI` and `LLMBuilder`, then bridges crate events to server `LlmEvent`; SessionActor does not depend on crate types or vendor wire data.
- The OpenAI API key is allowed in typed TOML but must never appear in Debug output, errors, logs, telemetry or client messages. Startup validates only local configuration and provider construction; it does not make a network health, quota, model-list or dummy-completion request.
- LlmRuntime is application-owned but not a native worker runtime. It owns shared LLM provider state, one global concurrency semaphore, runtime configuration and bounded event routing. Each LLM Operation is a fresh Tokio task scoped to Voice Session and generation, with CancellationToken and stream drop on cancel/timeout.
- An LLM permit and request timeout start when LlmRuntime accepts the operation and end only at Finished, Failed or Cancelled. Text deltas never reset the timeout. A terminal event that cannot reach its actor mailbox becomes controlled failure/cancellation, never a silently dropped completion.
- Phase 4 always invokes the crate stream with no tool definitions. Any returned tool call is Unexpected Tool Call: fail `llm_unexpected_tool_call`, cancel remaining LLM/SpeechOutput work, do not submit more Speech Segments, do not invoke MCP and do not retry. Audio already delivered cannot be recalled; queued or stale audio is blocked by GenerationGate. Tool-result rounds belong to Phase 6.
- Sentence Segmenter is a pure SpeechOutput delivery policy. Configuration contains `min_chars`, `soft_break_min_chars`, `max_chars` and `pending_segments`; punctuation is fixed for V1. Hard punctuation can flush at `min_chars`, soft punctuation at `soft_break_min_chars`, and `max_chars` forces a Unicode-safe split preferring whitespace.
- `pending_segments` defaults to 8 and is a hard semantic queue bound distinct from the worker command queue. Full pending capacity produces `speech_output_backpressure`, invalidates the generation, cancels the LLM Operation and stops accepting later deltas; it never drops, overwrites or skips a Speech Segment.
- Configure TTS as typed `zerotts_onnx`, Logical Model Identity `zerotts_default`, named voice `maichi`, thread count and timeout. TTS Model Preparation injects ResolvedModel into TtsFactory; the adapter receives no direct model paths and cannot acquire models itself.
- The ZeroTTS model pack pins roles for config, tokenizer, null voice, voice index, maichi voice latent, three TTS graphs, both codec decode graphs, codec shared data, codec metadata and codec license. The model-level license declaration is exactly `MIT; bundled-codec=Apache-2.0`, with an exact Model License Acknowledgement. Phase 4 does not migrate to artifact-level license policy.
- ZeroTTS has two deliberately separate native validation gates. The synthesis-core gate runs `text_encoder`, `prefix_step` and `local_frame_decode` with the pinned `maichi` latent, deterministic draws and a checked-in, artifact-derived token/code checkpoint. The worker-delivery gate adds both codec graphs and shared data, then runs the deterministic non-user `maichi` warmup before bind. Warmup succeeds only with terminal non-empty finite mono PCM at 48 kHz. It does not create a Voice Session, SpeechOutput, GenerationGate, Opus packet or WebSocket activity, and its mutable state is not reused as user work.
- ZeroTtsProvider returns only typed PcmF32Mono at 48 kHz mono. The adapter converts codec stereo to normalized mono; SpeechOutput alone resamples to Pcm16Mono 24 kHz, frames 1,440 samples, encodes canonical 60 ms Opus and paces delivery.
- TtsWorkerRuntime is application-owned and bounded. It owns native mutable ZeroTTS inference, worker command queues, timeout, cancellation/cleanup acknowledgement and quarantine. It must not run ONNX inference on the Tokio executor. A non-preemptible ONNX call may observe cancellation only at a safe point, but no late output may cross GenerationGate and its slot is reusable only after cleanup acknowledgement.
- `limits.tts_concurrency` is the sole application admission semaphore and must equal `workers.tts.max_workers`, the native structural capacity. Admission occurs before native worker lease; mismatched values are a startup validation failure.
- For each generation, SpeechOutput permits at most one active synthesis and preserves continuous ordinals in its bounded pending queue. Segment N+1 dispatches after SegmentFinished(N); its inference may overlap pacing of N only when audio queue bounds and playback order remain intact.
- TTS timeout begins when TtsWorkerRuntime accepts a segment, not when it is submitted or on each PCM chunk. Terminal SegmentFinished, Failed or cancelled acknowledgement ends it. Timeout or queue/worker admission failure fails the generation, invalidates GenerationGate, requests cleanup and quarantines the worker if cleanup grace expires. There is no logical TTS retry.
- SpeechOutput emits Drained only after FinishInput, empty pending queue, no active synthesis and paced final audio. SessionActor sends `tts:start` only after the first valid AudioPacket, owns `tts_started`/`tts_stopped` state per generation, and is the only outbound producer. A normal Drained emits one `tts:stop`, commits Delivered Assistant Response and releases Active Turn.
- Failure before Started sends neither playback control. Failure after Started invalidates the generation before cancellation, drops stale queued audio, sends exactly one `tts:stop`, and never commits Delivered Assistant Response. No binary audio for that generation may pass the writer after stop.

## Testing Decisions

- The primary deterministic seam is the existing application/router boundary with injected ProviderSet. Tests observe Voice Session phase, client-visible control/audio ordering, Conversational Turn outcome, Dialogue History commit and resource release rather than factory internals, native ONNX state or crate/vendor implementation details.
- Deterministic fake LLM and fake TTS providers cover ASR-final to LLM-start behavior, token-to-Speech-Segment flow, Unicode-safe segmenter policy, Unexpected Tool Call, remote LLM failure, timeout, cancellation, bounded event routing, global admission and pending-segment backpressure.
- SpeechOutput integration tests cover one active synthesis per generation, ordinal ordering with inference/pacer overlap, TTS timeout from worker acceptance, cleanup acknowledgement/quarantine, Drained preconditions, and the `tts:start`/first-packet/`tts:stop` wire ordering including failure-before-Started and failure-after-Started.
- Config/loader tests cover typed OpenAI and ZeroTTS selection, exact `llm` dependency feature expectations, no-network OpenAI startup, rejected capacity mismatch, validated SpeechOutput bounds, Model License Acknowledgement, missing ZeroTTS artifact roles and deterministic warmup failure conditions.
- Real local-model tests use the complete ZeroTTS pack to validate the three TTS graphs against pinned token/code checkpoints, then validate warmup and real PCM -> resample -> canonical Opus -> pacing. The test harness receives the verified installed pack and configured ONNX Runtime explicitly; a missing pack/runtime may make the test unavailable on an ordinary developer machine, but it must never be counted as passed, skipped CI proof, or Phase Completion evidence. Fixtures contain only pinned non-user text, token IDs, deterministic draws and compact code/checksum checkpoints; they must not expose warmup content or generated audio in telemetry.
- The mandatory Phase Completion Gate is a Reference Client end-to-end scenario through the public application boundary: valid ASR final, fake streaming LLM, real ZeroTTS, 24 kHz mono Opus decodable by the Reference Client, first audio before fake LLM Finished, `tts:start` before first audio, Drained then `tts:stop`, and no stale queued audio after cancellation or Unexpected Tool Call. It also includes a clean repository quality gate (`cargo fmt --check`, workspace tests and all-target Clippy with warnings denied); an ignored local-model test cannot substitute for this gate.
- A live OpenAI invocation is opt-in deployment smoke only. It verifies real configuration against the external service but is not a CI or Phase Completion Gate.
- Reuse existing protocol state, manual-STT, configuration/audio, provider-registry and Reference Client prior art; extend their public assertions rather than testing private queues, ORT tensors or crate internals.

## Out of Scope

- Device MCP execution, passing tool definitions to LLM, tool-result loops, tool-depth behavior and LLM-visible Tool policy; these begin in Phase 6.
- Custom OpenAI HTTP/SSE parsing, OpenAI TTS, Python sidecars, ZeroTTS HTTP service and HTTP fallback.
- Dynamic provider plugins, runtime code discovery, hot model swap, lazy model download during Voice Session and automatic retry of a logical provider operation.
- Acoustic barge-in, AEC, VAD while Speaking, new WebSocket message types, new audio profiles, persistent/cross-session Dialogue History, transcript/audio logging and voice cloning.
- Artifact-level licensing migration, except retaining the required codec license artifact and model-level composite acknowledgement.
- Treating a live OpenAI call, a build, or fake-provider-only tests as proof of the real ZeroTTS/Reference Client Phase Completion Gate.

## Further Notes

- This spec extends the accepted provider, SpeechOutput, bounded-concurrency, delivery-commit, canonical-audio, no-retry, compile-time-registry and pinned-model-preparation ADRs. It does not reopen them.
- Phase 3 remains a prerequisite: the existing current-generation non-empty AsrFinal and Active Turn boundary are the sole entry to Phase 4 delivery.
- The phase should be implemented in focused tickets, but no ticket can claim Phase 4 complete until the mandatory real-ZeroTTS Reference Client gate passes. Live OpenAI smoke remains separately reported.
- Ticket 06 must explicitly authorize the direct ndarray/NPZ reader dependency needed to parse the pinned `maichi` latent and must not replace it with a Python helper or a hand-written latent. Ticket 07 owns codec decoding and actual startup warmup; Ticket 06 does not claim PCM output or factory warmup.
- Ticket 08 closed the Phase 4 real-model Reference Client gate on 2026-09-22. The current shell may still lack `VOICE_ONNX_RUNTIME_LIB`; that makes an opt-in rerun unavailable here, not the completed gate invalid.

## Compatibility Follow-up

Ticket 09 extends V1 text-turn interoperability without redefining Phase 4 completion. `listen:detect { text }` is a second ingress source that joins the existing accepted-user-text boundary before LLM/SpeechOutput. It must preserve the completed ASR-final and delivery contracts.

- V1 accepts missing or empty inbound `session_id` for compatibility. A present non-string field is `Unknown`; a present non-empty string must exactly match the Voice Session or the command is a side-effect-free no-op.
- `Detect` is accepted only in Manual + Listening. It trims text, rejects empty or more than 4,096 Unicode scalars without mutation, keeps the generation created by `listen:start`, and admits the Active Turn before committing history or best-effort `stt`.
- A valid Detect detaches semantic ASR ownership immediately, dispatches cancellation, and tracks its cleanup identity independently. Late ASR final may never create another turn; any pending cleanup timeout remains fail-closed.
- The Reference Client gains a strict `send-text` public-boundary gate: validated ServerHello/session identity, canonical downlink Opus decoding, correlated `tts:start`/`tts:stop`, absolute deadlines, and a post-stop quiet period. Its deterministic fake-provider route is mandatory; real ZeroTTS, live OpenAI, and `py-xiaozhi` are opt-in smokes.
