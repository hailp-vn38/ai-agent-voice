# 14: Nghiệm thu hardware với nhiều lịch và lỗi mạng/nguồn

**What to build:** ESP32 xử lý lịch vượt cache window và phục hồi khi mất Wi-Fi hoặc nguồn, không trộn audio hay wake loop.

**Blocked by:** 13: ESP32 dùng một timer để wake/reconnect và phát.

Status: ready-for-agent

- [ ] 100 Reminder/window 64 có refill đúng; 64/65 boundary và năm same-time due một socket/năm sequential playback/individual drain ACK, một timer.
- [ ] Power cut giữa NVS write/reboot không phá cache lành; startup time validity/reconnect/sync và lease recovery server phối hợp đúng.
- [ ] Mất Wi-Fi mười phút, stale overdue, clock skew, output error, result/ACK mất và reconnect thay session có bounded backoff/dedupe; không fake completed.
- [ ] Create/cancel/complete/expire races vẫn current schedule ACK trước safe sleep hoặc explicit backoff; cache ACK global không biến out-of-window thành acked.
- [ ] Ghi firmware build/repo/commit/hardware profile/calibration và logs từng gate; failures/NOT_RUN có lý do, không báo hardware verified chỉ từ harness.

## Comments

2026-10-12: Người dùng chỉ đạo triển khai Web/server trước. Ticket firmware/phần cứng này được để mở cho giai đoạn sau; không dùng server harness để thay bằng chứng NVS/audio sink/RTC thật.
