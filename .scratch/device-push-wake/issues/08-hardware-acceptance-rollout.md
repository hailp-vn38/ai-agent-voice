# 08: Nghiệm thu phần cứng và bật trên production

**What to build:** Cấp chứng chỉ TLS và mở cổng cho broker production, bật tính năng, đo và ghi lại hành vi trên ESP32 thật.

**Blocked by:** 03: Mosquitto trong các bộ compose; 04: `reminder_scheduler` phát Wake Signal cho Device không có phiên; 06: Firmware phát Reminder và gửi Playback ACK; 07: Trạng thái đồng bộ `wake_on_due` cho Admin.

Status: ready-for-human

Spec: [device-push-wake](../spec.md) · [ADR 0145](../../../docs/adr/0145-device-push-channel-mqtt-wake-signal.md)

- [ ] DNS, chứng chỉ TLS và firewall cho cổng MQTT công khai; secret Device Push đặt trong môi trường production.
- [ ] Bật tính năng; xác nhận Device đang chạy nhận khối `push` ở lần khởi động kế tiếp.
- [ ] Đo độ trễ từ lúc đến hạn tới lúc bắt đầu phát trên phần cứng thật, ít nhất 10 lần; ghi số đo vào Comments.
- [ ] Kịch bản mất mạng: ngắt Wi-Fi qua giờ hẹn, nối lại, Reminder phát trễ đúng một lần.
- [ ] Kịch bản chặn Device: chặn trên giao diện quản trị, Device không kết nối lại được broker.
- [ ] Kịch bản broker dừng: thoại vẫn hoạt động, server tự nối lại khi broker lên.
- [ ] Wake word vẫn hoạt động trong lúc giữ Device Push Channel; ghi mức RAM trống trên bo ít bộ nhớ nhất được hỗ trợ.
- [ ] Chuyển ADR 0145 sang Accepted kèm trạng thái đã kiểm chứng, hoặc ghi lại điểm chưa đạt.

## Comments
