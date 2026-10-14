# 08: Phục hồi, retry bounded và hết hạn Reminder

**What to build:** Reminder lỗi hoặc uncertain được thử lại có giới hạn qua restart, không mất retry quota khi offline; Admin thấy lý do và trạng thái thực.

**Blocked by:** 07: Reminder đến hạn phát online và hoàn tất bằng Playback ACK.

Status: ready-for-agent

- [ ] Failed/uncertain attempts dùng bounded exponential backoff, tổng attempts_used không reset; mỗi attempt là logical TTS operation mới không retry bên trong provider.
- [ ] Claim chưa start recover/release không trừ quota; started mất WS/ACK uncertain, startup lease recovery và immediate due scan thực sự chạy từ production process.
- [ ] Operator limits retry=3/lateness=86.400.000 ms default được validate; hết quota/grace expired + revision tăng + sync hint sau commit; attempt telemetry không tăng schedule.
- [ ] ACK deadline bound theo audio duration/sink latency và hard cap, utterance dài không bị retry sớm; old ACK/duplicate completion không tiêu quota hoặc complete attempt mới.
- [ ] Device Detail hiển thị late/offline/uncertain/expired; privacy-safe attempt correlation và retry/expiry metrics không text/ID labels.
- [ ] Controlled process restart/fake clock/TTS tests chứng minh offline một giờ không tăng attempts, claim crash, started crash, WS lost after stop, exhaustion, concurrent claim và shutdown.
- [ ] Chốt bounded attempt/key retention với cửa sổ replay; không xóa persisted schedules để rollback hoặc tuyên bố exactly-once playback.
