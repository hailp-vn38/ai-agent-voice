# 01: Chuẩn hóa Direct Speech và giữ Voice/MCP tương thích

**What to build:** Prefactor nhỏ đường MCP Direct TTS hiện hữu thành Direct Speech dùng chung, kiểm chứng bằng Voice WS trước khi mở caller mới.

**Blocked by:** None (can start immediately).

Status: resolved

- [x] Giữ nguyên public behavior Voice/MCP của client cũ, admission, writer terminal outcome và cancellation; không mở feature mới mặc định.
- [x] Phân biệt nguồn speech trong actor; nội dung LLM vẫn phát llm control đúng, caller mới có thể dùng TTS controls mà không tạo hội thoại giả.
- [x] Tái sử dụng SpeechOutput, Active Turn permit, generation fence và Opus 24 kHz mono 60 ms; không thêm pipeline/runtime/WS thứ hai.
- [x] WS regression qua provider doubles chứng minh playable-first start, audio/stop ordering, abort và lỗi trước/sau first packet; không kiểm tra private fields.
- [x] Chốt golden contracts cho feature opt-in, session/request/snapshot/attempt identity và typed outcomes theo spec; fixtures thiết kế chưa được coi là production implementation.

## Answer

Shared `SpeechOrigin`/`DirectSpeechRequest` metadata and `begin_direct_speech` reuse the admitted turn, SpeechOutput, TTS worker, generation gate and sole writer. LLM text remains `llm`; direct tool text now uses `tts:sentence_start` after playable start and stays out of assistant history. This is the explicit ADR-0088 origin correction; MCP dispatch and audio behavior remain intact. No new caller or feature is enabled.

Public WS provider-double regressions cover canonical 24 kHz mono 60 ms packets, start/sentence/audio/stop order, pre/post-first-packet provider failure, abort, generation fencing, queue pressure and transcript privacy. TDD red: direct tool control assertion failed before the origin correction; the final public tests pass.

Validation: `cargo fmt --check`; `cargo clippy --all-targets --all-features -- -D warnings`; `cargo test --workspace` all passed against the final code. Workspace evidence includes all 16 `speechoutput_tracer`, 12 `tool_round_executor` and 15 `transcript_archive` tests. Logs: `/tmp/voice-reminders-01-{clippy-final,workspace}.log`.

Canonical [golden design fixtures](../../../crates/voice-agent-server/tests/fixtures/voice_reminders_protocol_v1.json) and [protocol contracts](../../../docs/voice_reminders_protocol_contracts.md) bind feature defaults, session/request/snapshot/attempt identities and typed outcomes. These fixtures explicitly are design contracts, not evidence of implemented Reminder/Direct TTS adapters.

Firmware/hardware verification is deferred by the user's server/Web-first scope. Integration base: `integration/voice-reminders-direct-tts` at `2092357` (merged before final validation).
