# Migration `memory_mode` và backup

Type: task
Status: ready-for-agent
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
