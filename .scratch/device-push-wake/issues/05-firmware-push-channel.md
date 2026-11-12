# 05: Firmware giữ Device Push Channel và mở phiên khi có Wake Signal

**What to build:** ESP32 đọc khối `push` từ OTA, giữ kết nối MQTT, và mở Voice Session không bật mic khi nhận Wake Signal.

**Blocked by:** 01: OTA cấp thông tin Device Push Channel cho Device đã đăng ký; 03: Mosquitto trong các bộ compose.

Status: ready-for-agent

Spec: [device-push-wake](../spec.md) · [ADR 0145](../../../docs/adr/0145-device-push-channel-mqtt-wake-signal.md)

- [ ] Đọc `firmware/esp32/AGENTS.md` trước; làm trong cây firmware thật, build thật.
- [ ] Parse khối `push` của phản hồi OTA vào vùng cấu hình riêng; không đụng tới xử lý khoá `mqtt` và `websocket`. Không có khối `push` thì Device chạy như hiện tại.
- [ ] Lớp Device Push Channel tách khỏi tầng giao thức thoại: kết nối, subscribe topic được cấp, tự nối lại có backoff. Xác nhận client MQTT của bo dùng TLS trên cổng production; ghi kết quả vào Comments.
- [ ] Wake Signal hợp lệ, chưa hết hiệu lực, khi không có phiên: mở kênh âm thanh WebSocket với hello khai báo reminders, không bật mic, không vào trạng thái nghe.
- [ ] Bỏ qua tín hiệu sai `v`, sai `type`, đã quá `expires_at`, hoặc khi đang có phiên.
- [ ] Ngắt Device Push Channel khi mở Voice Session và nối lại khi phiên đóng.
- [ ] Broker không truy cập được: Device khởi động, wake word và trò chuyện vẫn bình thường.
- [ ] Log phần cứng thật cho: kết nối, nhận tín hiệu retained sau khi khởi động lại, mở phiên do Wake Signal.

## Comments
