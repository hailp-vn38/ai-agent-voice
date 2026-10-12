# 03: Client yêu cầu Direct TTS trên Voice Session

**What to build:** Client đã được cấp quyền gửi tts:speak và nhận accepted/rejected rồi audio trên chính session đó.

**Blocked by:** 01: Chuẩn hóa Direct Speech và giữ Voice/MCP tương thích.

Status: ready-for-agent

- [ ] Feature direct_tts mặc định false, policy default deny; feature flag không tự cấp grant direct_tts.speak.
- [ ] Validate authenticated current session, request ID, text/rate/capacity; payload không tự chọn Device/provider/Agent/source.
- [ ] Busy reject, không tạo queue offline; metadata/source/status chỉ gửi client opt-in, errors sau accepted có terminal outcome rõ.
- [ ] WS golden tests chứng minh ASR=0/LLM=0/TTS=1 và start/binary/stop đúng; malformed/unknown legacy messages vẫn theo protocol fault boundary.
- [ ] Abort/security invalidation/generation fencing và before/after first-audio failures giữ đúng hiện hữu; không nhầm accepted với played.
