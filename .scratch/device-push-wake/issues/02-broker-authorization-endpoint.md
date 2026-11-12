# 02: Server trả lời câu hỏi phân quyền của broker

**What to build:** Endpoint HTTP để Mosquitto hỏi về quyền kết nối và quyền topic, do `device_push` quyết định dựa trên HMAC và Device Admission.

**Blocked by:** 01: OTA cấp thông tin Device Push Channel cho Device đã đăng ký.

Status: ready-for-agent

Spec: [device-push-wake](../spec.md) · [ADR 0145](../../../docs/adr/0145-device-push-channel-mqtt-wake-signal.md)

- [ ] Xác nhận định dạng yêu cầu và mã trả về của plugin xác thực HTTP cho Mosquitto theo tài liệu chính thức của plugin trước khi chốt hình dạng endpoint; ghi nguồn vào phần Comments.
- [ ] Xác thực Device: chấp nhận khi mật khẩu khớp HMAC và Device đã đăng ký; từ chối khi mật khẩu sai, Device bị chặn hoặc không tồn tại.
- [ ] Quyền topic của Device: chỉ đọc/subscribe topic `signal` của chính nó; mọi publish và mọi topic khác bị từ chối.
- [ ] Tài khoản dịch vụ của server: xác thực bằng mật khẩu dẫn xuất từ secret; chỉ được publish lên các topic `signal`. Câu hỏi superuser luôn bị từ chối.
- [ ] Endpoint chỉ trả cho phép/từ chối, không lộ lý do; không tồn tại khi tính năng tắt.
- [ ] Test đầu-cuối: thông tin lấy từ phản hồi OTA của một Device qua được endpoint xác thực và quyền topic; chặn Device rồi hỏi lại thì bị từ chối.
- [ ] Lỗi database khi tra admission dẫn tới từ chối, không cho qua.

## Comments
