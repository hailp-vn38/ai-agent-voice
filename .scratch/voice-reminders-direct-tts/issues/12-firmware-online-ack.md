# 12: Firmware thật đồng bộ NVS và ACK playback online

**What to build:** ESP32 thật nhận lịch, ghi NVS rồi ACK; phát reminder online, drain audio sink rồi ACK và đồng bộ terminal revision.

**Blocked by:** 07: Reminder đến hạn phát online và hoàn tất bằng Playback ACK.

Status: ready-for-agent

- [ ] Xác định actual firmware repo/commit/hardware profile, không fake cây firmware trong server repo; build thật và ghi hardware logs.
- [ ] Firmware opt-in, sync_request mỗi reconnect, sync_required fetch; bounded versioned NVS snapshot tối đa 64 items, native atomic durable write/verify trước ACK.
- [ ] Write failure giữ cache lành, sync_failed/retry; chỉ two-slot nếu storage recovery evidence yêu cầu. Không chủ động đóng WS khi current schedule chưa ACK hoặc chưa có safe backoff.
- [ ] End-of-stream stop không là played; decode/I2S/DAC/DMA buffers drain mới gửi đúng attempt ACK, output/decode/abort lỗi gửi failed.
- [ ] Đợi delivery_result đúng attempt, ACK terminal/current snapshot rồi mới cho idle sleep; dedupe recent attempt và duplicate ACK an toàn.
- [ ] Hardware online create/update/cancel → cache ACK → due playback → drain ACK → completed → terminal sync được chứng minh; old firmware regression không controls mới.

## Comments

2026-10-12: Người dùng chỉ đạo triển khai Web/server trước. Ticket firmware/phần cứng này được để mở cho giai đoạn sau; không dùng server harness để thay bằng chứng NVS/audio sink/RTC thật.
