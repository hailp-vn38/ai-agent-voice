# 02: GenerationGate và urgent explicit abort

**What to build:** Khi assistant đã Started, Voice Protocol Client nhận đúng một `tts:stop` sau explicit abort và không nhận thêm bất kỳ turn payload stale nào; nếu stop khẩn không admission được, Voice Session fail-closed thay vì tiếp tục playback không xác định.

**Blocked by:** 01 — Offline model preflight và fixture Opus Phase 5.

**Status:** resolved

- [x] Shared GenerationGate chặn cả JSON turn-scoped lẫn Opus stale tại writer sau interruption linearization point; payload đã thực sự send trước point không bị tuyên bố là có thể thu hồi.
- [x] Writer ưu tiên bounded `urgent > normal control > audio`; interrupt stop sau Started không phụ thuộc normal control queue và admission failure kích hoạt fail-closed escape path.
- [x] Deterministic public application test gây pressure normal control/audio, gọi abort, và quan sát stop/order/no-stale thay vì chọc vào queue private.

## Comments

- Hoàn tất 2026-09-23: `GenerationGate` là boundary chung actor/writer cho `TurnText` và canonical Opus. Writer có lane urgent bounded, ưu tiên urgent trước normal control và audio. Nếu không admit được interruption stop thì actor root-cancel, writer shutdown fail-closed.
- Xác minh: `cargo test -p voice-agent-server --test speechoutput_tracer --test manual_stt --test config_audio`, `cargo check -p voice-agent-server` pass. Full workspace gate được chạy trước commit.
