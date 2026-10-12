# 02: Thử Direct TTS trên Device từ Admin Web

**What to build:** Admin nhập text ở Device Detail, gửi yêu cầu và phát qua đúng Voice Session đang hoạt động; UI phân biệt accepted với played.

**Blocked by:** 01: Chuẩn hóa Direct Speech và giữ Voice/MCP tương thích.

Status: ready-for-agent

- [ ] Active Device Sessions register sau admission/actor init; unregister có token fence, replacement cũ không gỡ handle mới; không giữ lock qua await.
- [ ] Admin-authenticated speak route validate text 1–1.024 Unicode characters/4.096 UTF-8 bytes, rate và capacity; actor queued mới trả 202.
- [ ] Chỉ dùng TTS profile đã admit, ASR=0/LLM=0/TTS=1; không stt/llm/history giả, không provider/source override từ payload.
- [ ] Offline/busy/unsupported/unauthorized/capacity lỗi typed; ephemeral request không được lưu chờ reconnect.
- [ ] Device Detail có Direct TTS Test với loading/error/i18n/keyboard; presence authoritative, enabled không là online và accepted không là phát xong.
- [ ] HTTP + WS + Web tests bao phủ security, replacement, managed-template boundary, shutdown và writer backpressure; legacy controls/metadata giữ tương thích.
