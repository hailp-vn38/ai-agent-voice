# 03: Mosquitto trong các bộ compose

**What to build:** Dịch vụ Mosquitto kèm plugin xác thực HTTP trỏ về server, bật theo profile, cho dev, test và production.

**Blocked by:** 02: Server trả lời câu hỏi phân quyền của broker.

Status: ready-for-agent

Spec: [device-push-wake](../spec.md) · [ADR 0145](../../../docs/adr/0145-device-push-channel-mqtt-wake-signal.md)

- [ ] Thêm dịch vụ broker vào các bộ compose dưới một profile; không bật profile thì bộ compose chạy như hiện tại.
- [ ] Broker không cho kết nối ẩn danh, không có danh sách người dùng cục bộ, mọi quyết định xác thực và quyền topic đi qua endpoint của ticket 02.
- [ ] Dev/test: listener không TLS trong mạng nội bộ compose. Production: listener TLS trên cổng công khai, đường dẫn chứng chỉ lấy từ cấu hình triển khai.
- [ ] Server kết nối broker qua tên dịch vụ nội bộ; cấu hình mẫu và tài liệu triển khai được cập nhật.
- [ ] Bộ compose qua được kiểm tra hiện có của feature docker-compose-validation.
- [ ] Kiểm tra khói với broker thật: tài khoản Device subscribe được topic của mình và bị từ chối topic khác; tài khoản dịch vụ publish được và Device nhận tin retained sau khi kết nối lại. Chạy ngoài bộ test mặc định; ghi kết quả vào Comments.

## Comments
