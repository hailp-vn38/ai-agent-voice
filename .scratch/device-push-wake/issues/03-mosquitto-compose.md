# 03: Mosquitto trong các bộ compose

**What to build:** Dịch vụ Mosquitto kèm plugin xác thực HTTP trỏ về server, bật theo profile, cho dev, test và production.

**Blocked by:** 02: Server trả lời câu hỏi phân quyền của broker.

Status: resolved

Spec: [device-push-wake](../spec.md) · [ADR 0145](../../../docs/adr/0145-device-push-channel-mqtt-wake-signal.md)

- [x] Thêm dịch vụ broker vào các bộ compose dưới một profile; không bật profile thì bộ compose chạy như hiện tại.
- [x] Broker không cho kết nối ẩn danh, không có danh sách người dùng cục bộ, mọi quyết định xác thực và quyền topic đi qua endpoint của ticket 02.
- [x] Dev/test: listener không TLS trong mạng nội bộ compose. Production: listener TLS trên cổng công khai, đường dẫn chứng chỉ lấy từ cấu hình triển khai.
- [x] Server kết nối broker qua tên dịch vụ nội bộ; cấu hình mẫu và tài liệu triển khai được cập nhật.
- [x] Bộ compose qua được kiểm tra hiện có của feature docker-compose-validation.
- [x] Kiểm tra khói với broker thật: tài khoản Device subscribe được topic của mình và bị từ chối topic khác; tài khoản dịch vụ publish được và Device nhận tin retained sau khi kết nối lại. Chạy ngoài bộ test mặc định; ghi kết quả vào Comments.

## Comments

- Broker: `iegomez/mosquitto-go-auth:3.0.0-mosquitto_2.0.18` (tag verified by pulling), profile `push` in `compose.yaml` (shared auth config, data volume), `compose.test.yaml` (plain 1883, compose-network only) and `compose.prod.yaml` (TLS 8883 published; internal plain 1883 for the server; cert/key from `MQTT_TLS_CERT`/`MQTT_TLS_KEY`). Config in `docker/mosquitto/`; auth mode per ticket 02 (`http`, json params, status responses, cache off).
- Compose validation: `docker compose config` for prod and test, with and without `--profile push`, covered by `scripts/tests/test_mosquitto_compose.py` (broker absent without profile, present with it, no published port in test, only 8883 in prod; no anonymous, no local user files) plus existing `test_compose_manager.py`.
- Smoke (`scripts/docker/mqtt-broker-smoke.sh`, outside default tests; real Mosquitto, auth endpoints replaced by `scripts/docker/mqtt_auth_stub.py` mirroring the server rules): all 9 checks passed: anonymous and wrong password refused, service publishes retained, Device receives it on first and reconnect, Device refused another Device's topic, Device cannot publish, service cannot subscribe. Prod config was also started with a self-signed cert and opened 8883 and 1883. Not done: smoke against the real server binary, and a TLS client handshake.
