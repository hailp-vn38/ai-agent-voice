# Task đọc: `memory_recall` và `memory_summary`

Type: task
Status: ready-for-agent
Blocked by: 04

## Scope

Mốc 6 của [spec](../spec.md), bước 3.5 và 3.4 trong [hướng dẫn](../../../docs/memory-core-implementation-guide.md).

- Hai `HostTool` chỉ đọc, luôn đăng ký, scope suy từ `ToolScope.owner_id`.
- Sửa nhánh `Primary` của `agent_settings`; `daily_root` thêm hai tool; `wiki_root` và Sub-Agent giữ nguyên.
- Mô tả tool tự đủ.

User stories: 70 đến 74.

## Acceptance

- **Test đầu tiên:** Task tạo qua `VoiceTaskSubmit` với `device:<id>` thì tool của `daily_root` thấy đúng `owner_id`. Nếu sai, dừng và báo lại trước khi làm tiếp.
- Mọi dòng mốc 6 trong ma trận test của spec đều có test qua.
- `scripts/agent-test.sh targeted task_flow voice-agent-server` xanh.
