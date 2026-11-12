# 06: Firmware phát Reminder và gửi Playback ACK

**What to build:** Trong phiên do Wake Signal mở (và cả phiên thường), ESP32 phát Direct Speech của Reminder, gửi Playback ACK sau khi audio sink drain, chờ kết quả rồi đóng phiên.

**Blocked by:** 05: Firmware giữ Device Push Channel và mở phiên khi có Wake Signal.

Status: ready-for-agent

Spec: [device-push-wake](../spec.md) · [ADR 0145](../../../docs/adr/0145-device-push-channel-mqtt-wake-signal.md)

- [ ] Hello khai báo khả năng reminders; firmware cũ không gửi thì server không gửi control mới.
- [ ] Phát âm thanh Direct Speech có nguồn gốc Reminder qua đường phát hiện có.
- [ ] Kết thúc luồng mạng không phải là đã phát: chỉ gửi `reminder.played` đúng Reminder và attempt sau khi bộ đệm decode và output đã drain.
- [ ] Lỗi decode, lỗi output hoặc bị huỷ giữa chừng gửi `reminder.failed`.
- [ ] Chờ `delivery_result` đúng attempt rồi mới đóng phiên do Wake Signal mở; có thời gian chờ tối đa. Nhiều Reminder đến hạn được phát tuần tự trong cùng phiên.
- [ ] Phiên do Wake Signal mở mà server không phát gì thì tự đóng sau thời gian chờ và Device quay lại chờ wake word.
- [ ] ACK trùng an toàn; không cache lịch, không ghi NVS cho Reminder, không hẹn giờ RTC.
- [ ] Bằng chứng phần cứng thật: tạo Reminder trên giao diện quản trị khi Device rảnh → Wake Signal → phiên → phát → ACK → Reminder `completed`.

## Comments
