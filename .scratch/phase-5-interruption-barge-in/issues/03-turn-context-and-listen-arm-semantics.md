# 03: TurnContext và listen arm semantics

**What to build:** Voice Session có Conversational Turn với CancellationToken riêng và một primitive interruption; `listen:start` chỉ arm/reset capture, còn explicit abort mới hủy turn, để client không vô tình cắt TTS chỉ vì điều khiển capture.

**Blocked by:** 02 — GenerationGate và urgent explicit abort.

**Status:** resolved

- [x] Turn mới luôn nhận token mới; token generation cũ không thể được un-cancel, và cancellation/cleanup/Active Turn release có thứ tự idempotent.
- [x] `listen:start` trong Processing/Speaking không tăng GenerationId, không invalidate outbound và không gọi cancellation delivery; Manual vẫn không nhận acoustic interruption.
- [x] Regression tại public session seam chứng minh abort, repeated listen arm, double interrupt và stale LLM/TTS event không revive old turn hoặc commit Delivered Assistant Response.

## Comments

- Hoàn tất 2026-09-23: `TurnContext` sở hữu một `CancellationToken` mới cho từng Conversational Turn và truyền token đó vào LLM runtime. `interrupt_active_turn` đặt gate trước cancel/release theo thứ tự idempotent. `listen:start` trong delivery chỉ ghi arm pending; sau terminal normal, actor khởi tạo capture mới mà không hủy hoặc invalidate turn cũ.
- Xác minh: `cargo test -p voice-agent-server --test llm_runtime --test speechoutput_tracer --test manual_stt`, `cargo check -p voice-agent-server`, `cargo fmt --check`, `git diff --check` pass. Full workspace gate chạy trước commit.
