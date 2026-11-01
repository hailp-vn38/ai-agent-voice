# Khởi tạo, Admin HTTP, UI và xóa Device

Type: task
Status: resolved
Blocked by: 02, 03

## Scope

Mốc 4 của [spec](../spec.md), bước 1.7 đến 1.9 trong [hướng dẫn](../../../docs/memory-core-implementation-guide.md).

- `AppState.memory_core`, mở độc lập với `wiki.enabled`; `reconcile_all` chạy nền lúc khởi động.
- Viết lại Admin handler `/memory/*` theo bảng route của spec, bỏ mọi lời gọi Wiki.
- Xóa Device purge vault sau khi `DELETE` thành công; đường import không purge.
- Trang Memory, form Device, cảnh báo xóa vault, chuỗi `memoryV3.*`.

User stories: 1 đến 8, 14 đến 22, 26 đến 29, 33, 68, 78.

Đây là ngoại lệ có chủ đích của spec: dữ liệu cũ trong `wiki/entities/` không còn hiện trên màn Memory.

## Acceptance

- Mọi dòng mốc 4 trong ma trận test của spec đều có test qua, gồm phần "Device đổi Agent vẫn giữ fact" của dòng Cách ly.
- `scripts/agent-test.sh targeted memory voice-agent-server`, `web test`, `web typecheck` xanh.

## Answer

Đã triển khai tại `f619d856`: `AppState.memory_core`, route `/memory/*`, purge vault sau khi xóa Device, trang Memory, form Device và chuỗi `memoryV3.*`. Các lỗi code review được sửa tại `15556c6a`; lỗi 503 có từ trước của `admin_api` được sửa tại `7c808c32`.
