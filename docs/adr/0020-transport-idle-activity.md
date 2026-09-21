# ADR 0020 — Transport idle đo activity WebSocket hai chiều

## Status
Accepted

V1 reset transport idle bằng bất kỳ WebSocket RX/TX hợp lệ, gồm ping/pong khi transport expose, với timeout server 300 giây; conversation idle bị tắt. Server không thêm heartbeat chỉ để né firmware baseline timeout khi không có server-to-device traffic, nên firmware có thể chủ động reconnect sau khoảng 120 giây im lặng.
