# 09: py-xiaozhi text-turn compatibility

**What to build:** Add V1 `listen:start(manual) -> listen:detect { text }` interoperability without reopening Phase 4 delivery lifecycle. The accepted Detect path joins the existing accepted-user-text boundary before LLM/SpeechOutput and the independent Reference Client gains a public-boundary `send-text` conformance flow.

**Blocked by:** 08: Reference Client Phase 4 completion gate.

**Status:** resolved

## Server Contract

- Parse `session_id` as envelope metadata on `Listen` and `Abort`, not as a `ListenCommand` field. Missing or `""` is accepted for V1 compatibility; a non-empty string must match the current Voice Session; a present non-string yields `ClientMessage::Unknown`. A mismatch is a no-op before every side effect.
- Accept Detect only when `phase == Listening` and mode is Manual. Normalize with Unicode trim; empty or more than 4,096 Unicode scalars is a silent no-op. Rejected commands do not mutate generation, phase, ASR ownership, Active Turn, history, LLM, TTS, or outbound controls.
- A valid Detect keeps the generation created by `listen:start`. It detaches semantic ASR ownership, dispatches `AsrCommand::Cancel`, then records the identity in `HashSet<WorkerIdentity>` cleanup tracking. Do not wait for Cancelled before LLM/TTS. Cancel dispatch failure is fail-closed and starts no text turn.
- Pending cleanup events are handled before current semantic ASR events. Late Final/Failed/Cancelled only clear their matching cleanup obligation; CleanupTimedOut or defensive FinalTimedOut fails closed even after a replacement generation began.
- After successful cancel dispatch, Active Turn admission is fail-fast. Denial returns to Ready without STT/history/LLM/TTS or retry, while retaining the cleanup obligation. Admission precedes commit; accepted Detect commits canonical text once and best-effort enqueues one STT, then reuses the existing LLM -> SpeechOutput -> ZeroTTS -> Opus path.
- Abort follows the same validate-if-present policy. A stale non-empty session ID must not cancel any operation, invalidate audio, release capacity, increment generation, or emit `tts:stop`.

## Reference Client Contract

- Add reusable `run_text_turn(TextTurnConfig) -> TextTurnReport` in the library; the `send-text` CLI command is only its Clap wrapper. Normalize local input before OTA: trim and reject empty or more than 4,096 Unicode scalars without network activity.
- Strictly parse the first expected ServerHello before sending Listen. Require `type=hello`, `transport=websocket`, non-whitespace string `session_id`, and downlink Opus 24 kHz mono 60 ms; ignore extra fields and do not require a server `version` field.
- Send matching non-empty session IDs on start and detect. ClientHello omits MCP capability advertisement until Phase 6.
- Require exact, non-empty matching session IDs on `tts:start` and `tts:stop`; missing, wrong-type, empty, or mismatch fails immediately. STT, sentence-start, unknown JSON, ping, and pong are auxiliary only and never reset deadlines.
- Validate every binary frame only after correlated `tts:start`: decode canonical Opus to exactly 1,440 samples. Require at least one packet before a correlated `tts:stop`; binary before start, duplicate start/stop, audio after stop, lifecycle after stop, or close/error during the quiet period fails.
- `TextTurnConfig` has positive `tts_start_timeout` (60 s default), `turn_timeout` (120 s default, not shorter than start timeout), and `post_stop_quiet_period` (250 ms default). All deadlines are absolute and do not reset from traffic. Expose seconds flags for the first two and `--post-stop-quiet-period-ms` for the last. Errors and observability use only state/counters/reasons, never text, IDs, credentials, JSON bodies, or audio.

## Required Evidence

- Public-router deterministic test uses the same Reference Client library flow with fake LLM and fake valid PCM TTS, but production OTA, WebSocket, SessionActor, SpeechOutput, resample, Opus encoder, and decoder. It proves ServerHello/session correlation, Detect starts exactly one turn, control/audio ordering, canonical decoding, and post-stop quiet behavior.
- Server regressions cover missing/empty/mismatching/wrong-type IDs; Manual/Listening-only acceptance; invalid text; stale Abort; late ASR Final after Detect; independent multi-identity cleanup; cleanup timeout; cancel dispatch failure; and Active Turn denial.
- `cargo fmt --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `git diff --check` pass.
- Real ZeroTTS, live OpenAI, and a pinned `py-xiaozhi` checkout are opt-in smoke only. Report PASS, FAIL, or NOT RUN separately; an unavailable environment does not block this ticket.

## Scope Boundaries

- Runtime `SessionPhase::Processing` continues to cover LLM and TTS delivery. ADR/docs describe Speaking only as a conceptual state; adding a runtime Speaking state is a separate refactor.
- Do not add a V2/strict session-ID mode, new wire error frames, MCP capability modeling, custom OpenAI HTTP/SSE, or writer reliability redesign in this ticket.

## Comments

- Design grill completed and confirmed 2026-09-22. The ticket records the accepted V1 compatibility contract for the current py-xiaozhi WebSocket behavior, including its empty session-id quirk.

- Hoàn tất 2026-09-22: V1 text-turn compatibility được nối vào public router, SessionActor và Voice Reference Client. Parser giữ session ID optional/rỗng cho V1, từ chối type sai hoặc ID stale trước side effect; Detect hợp lệ dùng accepted-user-text boundary, revoke semantic ASR ownership và vẫn theo LLM -> SpeechOutput -> canonical Opus hiện hữu. Client `send-text` xác minh ServerHello/session correlation, lifecycle `tts:start`/audio/`tts:stop`, Opus 24 kHz mono và quiet period.
- Xác minh: `cargo fmt --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `git diff --check` pass. Real ZeroTTS, live OpenAI và smoke với checkout `py-xiaozhi` không chạy trong lượt commit này (opt-in, không phải điều kiện đóng ticket).
