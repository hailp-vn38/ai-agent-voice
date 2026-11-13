# 02: Server trả lời câu hỏi phân quyền của broker

**What to build:** Endpoint HTTP để Mosquitto hỏi về quyền kết nối và quyền topic, do `device_push` quyết định dựa trên HMAC và Device Admission.

**Blocked by:** 01: OTA cấp thông tin Device Push Channel cho Device đã đăng ký.

Status: resolved

Spec: [device-push-wake](../spec.md) · [ADR 0145](../../../docs/adr/0145-device-push-channel-mqtt-wake-signal.md)

- [x] Xác nhận định dạng yêu cầu và mã trả về của plugin xác thực HTTP cho Mosquitto theo tài liệu chính thức của plugin trước khi chốt hình dạng endpoint; ghi nguồn vào phần Comments.
- [x] Xác thực Device: chấp nhận khi mật khẩu khớp HMAC và Device đã đăng ký; từ chối khi mật khẩu sai, Device bị chặn hoặc không tồn tại.
- [x] Quyền topic của Device: chỉ đọc/subscribe topic `signal` của chính nó; mọi publish và mọi topic khác bị từ chối.
- [x] Tài khoản dịch vụ của server: xác thực bằng mật khẩu dẫn xuất từ secret; chỉ được publish lên các topic `signal`. Câu hỏi superuser luôn bị từ chối.
- [x] Endpoint chỉ trả cho phép/từ chối, không lộ lý do; không tồn tại khi tính năng tắt.
- [x] Test đầu-cuối: thông tin lấy từ phản hồi OTA của một Device qua được endpoint xác thực và quyền topic; chặn Device rồi hỏi lại thì bị từ chối.
- [x] Lỗi database khi tra admission dẫn tới từ chối, không cho qua.

## Comments

- Plugin docs source: mosquitto-go-auth README, section "HTTP" (https://github.com/iegomez/mosquitto-go-auth). Chosen: `auth_opt_http_params_mode json`, `auth_opt_http_response_mode status` (2xx allows, otherwise denied), POST. Bodies: user `{username,password,clientid}`, superuser `{username}`, acl `{username,clientid,topic,acc}` with acc 1=read, 2=write, 3=readwrite, 4=subscribe. Endpoints: `/mqtt/auth/user`, `/mqtt/auth/superuser`, `/mqtt/auth/acl` (200 allow, 403 deny, empty body). Mosquitto config for these belongs to ticket 03.
- Service account: username `vela-server`, password = hex HMAC-SHA256(secret, "v1-service:vela-server").
- Device ACL re-checks admission, so a blocked Device is denied even if the plugin caches nothing. Tests: `crates/vela-server/tests/device_push_auth.rs`.
