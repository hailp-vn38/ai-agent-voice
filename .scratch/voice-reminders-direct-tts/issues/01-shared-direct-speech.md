# 01: Chuẩn hóa Direct Speech và giữ Voice/MCP tương thích

**What to build:** Prefactor nhỏ đường MCP Direct TTS hiện hữu thành Direct Speech dùng chung, kiểm chứng bằng Voice WS trước khi mở caller mới.

**Blocked by:** None (can start immediately).

Status: ready-for-agent

- [ ] Giữ nguyên public behavior Voice/MCP của client cũ, admission, writer terminal outcome và cancellation; không mở feature mới mặc định.
- [ ] Phân biệt nguồn speech trong actor; nội dung LLM vẫn phát llm control đúng, caller mới có thể dùng TTS controls mà không tạo hội thoại giả.
- [ ] Tái sử dụng SpeechOutput, Active Turn permit, generation fence và Opus 24 kHz mono 60 ms; không thêm pipeline/runtime/WS thứ hai.
- [ ] WS regression qua provider doubles chứng minh playable-first start, audio/stop ordering, abort và lỗi trước/sau first packet; không kiểm tra private fields.
- [ ] Chốt golden contracts cho feature opt-in, session/request/snapshot/attempt identity và typed outcomes theo spec; fixtures thiết kế chưa được coi là production implementation.
