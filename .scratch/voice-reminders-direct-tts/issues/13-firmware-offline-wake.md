# 13: ESP32 dùng một timer để wake/reconnect và phát

**What to build:** ESP32 có lịch đã ACK đóng WS, wake trước hạn, reconnect/revalidate, phát đúng hạn rồi cập nhật lịch trước ngủ lại.

**Blocked by:** 08: Phục hồi, retry bounded và hết hạn Reminder; 12: Firmware thật đồng bộ NVS và ACK playback online.

Status: ready-for-agent

- [ ] Một logical timer earliest future due trừ measured connection margin/next sync/local backoff; UTC/server offset và monotonic/RTC có reset/overflow validation.
- [ ] Wake → Wi-Fi/auth/same Device WS → sync_request/revalidate; giữ connection từ wake sớm đến due/delivery hoặc bounded failure policy, không phát sớm và không socket thứ hai.
- [ ] Sau played phải delivery_result + terminal/current snapshot ACK + recompute future timer; overdue/unreachable bounded backoff, không deadline quá khứ/0ms loop.
- [ ] Software timer khi cần wake word; Deep Sleep chỉ hardware/profile cho phép, measured margin/sink signal được ghi rõ.
- [ ] Cancel khi Device ngủ có thể wake thừa nhưng không phát; Web create lúc ngủ UI sync_pending không hứa đúng giờ; firmware chỉ cache wake metadata không offline TTS.
- [ ] Real hardware gate chứng minh cache ACK → WS close → timer → authenticated reconnect → due Direct Speech → sink drain ACK → terminal sync → future wake và wake-word vẫn hoạt động.
