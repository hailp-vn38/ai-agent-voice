# ADR 0008 — Trusted LAN dùng static Bearer token tùy chọn

## Status
Accepted

V1 triển khai trong trusted LAN và dùng một static token toàn server: `auth.token = ""` tắt authentication, còn giá trị không rỗng bắt buộc `Authorization: Bearer <token>` và thiếu/sai token bị từ chối với HTTP 401. Device ID và Client ID chỉ là metadata; V1 không có credential theo thiết bị, token database, revoke hoặc rotate theo thiết bị. OTA trả token này khi nó được bật, nên OTA không phải security boundary.

## Consequences

Internet không nằm trong supported V1 profile; nếu tự triển khai qua Internet/VPN thì cần WSS/reverse proxy/VPN và bật token. Mỗi Device ID chỉ có một Voice Session; kết nối mới thay thế, cancel và đóng session cũ.
