# Migration `memory_mode` và backup

Type: task
Status: resolved
Blocked by: 01

## Scope

Mốc 3 của [spec](../spec.md), bước 1.6 trong [hướng dẫn](../../../docs/memory-core-implementation-guide.md).

- Migration 0141 cho `devices.memory_mode`.
- `memory_binding(db, device_db_id)`; `memory_mode` đi qua Device read model, Admin API, backup export, import và inspector.
- Giữ `BUNDLE_SCHEMA_VERSION = 1`.
- Kiểu `api/devices.ts` của Admin web nhận và trả `memory_mode`. Form Device thuộc ticket 04.

User stories: 20 (phần API), 30 đến 32, 81.

## Acceptance

- Mọi dòng mốc 3 trong ma trận test của spec đều có test qua.
- `scripts/agent-test.sh targeted backup voice-agent-server` xanh, thêm `web typecheck` nếu đụng `apps/admin-web`.

## Answer

Đã triển khai tại `dc38a10f`: migration 0141, `memory_binding`, `memory_mode` qua read model, Admin API và backup. `#[serde(default)]` không đủ vì checksum serialize lại bundle, nên giá trị `off` được bỏ khỏi bundle khi export; snapshot SQLite cũ thiếu cột cũng đọc thành `off`. Các lỗi code review được sửa tại `15556c6a`; lỗi 503 có từ trước của `admin_api` được sửa tại `7c808c32`.
