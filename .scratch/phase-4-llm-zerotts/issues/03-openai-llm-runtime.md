# 03: OpenAI adapter và LLM Operation runtime

**What to build:** Application dùng adapter `openai` qua crate Rust `llm` pin exact để chạy một LLM Operation bounded theo Voice Session/generation. Startup chỉ validate/build local; failure remote, timeout, cancel và Unexpected Tool Call fail đúng generation mà không tạo MCP call, retry hoặc server-wide outage.

**Blocked by:** 02: Synthetic SpeechOutput tracer bullet.

**Status:** resolved

- [x] LLM adapter bridge typed events của crate thành domain LlmEvent mà SessionActor không import type/vendor wire data; default crate features không được kéo vào server.
- [x] LlmRuntime giữ global admission, per-operation timeout từ accept đến terminal event, CancellationToken/drop stream và terminal-event routing fail-closed.
- [x] Contract/application tests chứng minh startup không network probe, remote error chỉ ảnh hưởng current generation, cancel release permit sau task terminal, và tool event bất ngờ cancel speech còn lại không MCP/retry.

## Comments

- Hoàn tất 2026-09-22: pin `llm =1.3.8` với default features tắt, chỉ `openai` và `rustls-tls`; factory build OpenAI local qua `LLMBuilder`, adapter chỉ bridge `chat_stream_with_tools(..., None)` sang `LlmEvent`. `LlmRuntime` application-owned giữ semaphore toàn app, timeout, cancellation và route event theo Voice Session/generation. Unexpected Tool Call, lỗi stream, timeout hoặc cancel kết thúc đúng operation, huỷ SpeechOutput và không có đường MCP/retry.
- Xác minh: `cargo check -p voice-agent-server`; `cargo test -p voice-agent-server --test llm_runtime --test speechoutput_tracer --test manual_stt --test protocol_e2e --test config_audio`; `cargo fmt`; `git diff --check` pass. `cargo test --workspace` vẫn bị chặn trước ticket 03 bởi test untracked ticket 05 `zerotts_artifact_preflight.rs` tham chiếu `WarmupPcm`/`validate_warmup_pcm` và chữ ký `TtsFactory` chưa được triển khai.
