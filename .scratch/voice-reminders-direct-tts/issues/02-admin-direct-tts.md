# 02: Thử Direct TTS trên Device từ Admin Web

**What to build:** Admin nhập text ở Device Detail, gửi yêu cầu và phát qua đúng Voice Session đang hoạt động; UI phân biệt accepted với played.

**Blocked by:** 01: Chuẩn hóa Direct Speech và giữ Voice/MCP tương thích.

Status: resolved

- [x] Active Device Sessions register sau admission/actor init; unregister có token fence, replacement cũ không gỡ handle mới; không giữ lock qua await.
- [x] Admin-authenticated speak route validate text 1–1.024 Unicode characters/4.096 UTF-8 bytes, rate và capacity; actor queued mới trả 202.
- [x] Chỉ dùng TTS profile đã admit, ASR=0/LLM=0/TTS=1; không stt/llm/history giả, không provider/source override từ payload.
- [x] Offline/busy/unsupported/unauthorized/capacity lỗi typed; ephemeral request không được lưu chờ reconnect.
- [x] Device Detail có Direct TTS Test với loading/error/i18n/keyboard; presence authoritative, enabled không là online và accepted không là phát xong.
- [x] HTTP + WS + Web tests bao phủ security, replacement, managed-template boundary, shutdown và writer backpressure; legacy controls/metadata giữ tương thích.

## Answer

Implemented Admin Direct TTS through a fenced active-Device-session registry. The authenticated `POST /api/admin/devices/{device_id}/speak` route admits only an enabled, online, Direct-TTS-capable physical Device and returns `202` only after its actor has queued the request. It preserves the admitted TTS profile, accepts no caller profile/source override, and returns typed offline, capability, capacity, rate, and session errors.

The Device Detail test control reports acceptance separately from playback. Direct speech reuses the bounded speech pipeline while bypassing ASR, LLM, transcript/history writes, and response-text filtering so Admin text reaches TTS unchanged. Session replacement and unregister paths use a session-token fence; lifecycle, actor, HTTP, writer-backpressure, Web, and exact-payload regressions cover the contract.

Validation used an isolated build cache: `cargo fmt --check`; `cargo clippy --all-targets --all-features -- -D warnings`; and `cargo test --workspace` all passed (the workspace suite passed with two existing environment-dependent tests ignored).
