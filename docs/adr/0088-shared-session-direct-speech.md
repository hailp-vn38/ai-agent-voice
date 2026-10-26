# ADR 0088 — Direct Speech dùng chung Voice Session và SpeechOutput

## Status

Accepted, 2026-10-26 (Proposed, 2026-10-12; mở rộng của ADR-0006 và ADR-0017).

Tổng quát đường MCP Direct TTS hiện có thành Direct Speech cho tool, client, Admin và Reminder, đi qua `SessionActor → SpeechOutput → TTS Worker → Opus → WebSocket Writer`. Chọn tái sử dụng pipeline thay vì tạo notification WS/TTS runtime thứ hai: giữ cancellation, pacing, backpressure và một chủ thể gửi audio.

- Server-originated speech đi qua typed mailbox event, không giả ClientMessage. Mỗi lần phát dùng TTS của Effective Session Profile đã admit, không chạy ASR/LLM, không commit user text hoặc thêm Dialogue History giả.
- Phân biệt Speech Origin; chỉ nội dung LLM mới phát `type:"llm"` tại `SegmentReady`. Direct Speech dùng TTS controls; metadata mới chỉ gửi cho client opt-in, giữ Opus 24 kHz mono 60 ms và writer/generation fencing hiện hữu.
- Giữ Active Turn admission/permit và terminal outcome; không gửi `tts:start` trước audio hợp lệ theo [ADR-0031](0031-tts-start-means-playable-audio.md). Một Device chỉ phát một speech turn; `SpeechOrigin::Reminder` được miễn trừ rate-limit 1 giây của Admin `/speak`, được bảo vệ bởi khóa `in_flight_reminder` trên phiên, và chờ tuần tự FIFO bounded theo `(due_at, id)` tại safe turn boundary (khi `SessionActor` trở về trạng thái `Listening` rảnh và đánh thức scheduler), không tự cắt lời. Admin/client Direct TTS bận thì reject, ephemeral và không lưu chờ khi offline.
- Application registry định tuyến theo Device/current session, đăng ký sau admission/actor init thành công; unregister phải khớp instance token để session cũ không xóa session mới, theo [ADR-0036](0036-initialize-replacement-session-before-displacing-incumbent.md). Không giữ registry lock qua await.

`accepted` chỉ là actor đã nhận/queue, không chứng minh socket delivery hay playback. Abort/security invalidation vẫn ưu tiên; mọi text, queue và frame giữ giới hạn hiện có hoặc hằng số validated của guide.

Nguồn: [Guide §3, §5–6, §9](../voice_reminders_direct_tts_implementation_guide.md#9-direct-tts-chính-sách-và-thực-thi-trong-actor).
