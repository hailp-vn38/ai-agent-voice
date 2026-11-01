# Flow 16 — Background Task và Multi-Kind Task Artifacts

Cập nhật: 2026-11-01. Quyết định nền tảng: [ADR 0127](../adr/0127-orchestrator-flow-task-and-wiki-runtime.md). Web timeline: [Flow 09](09-agent-job-web-timeline.md).

## Khởi tạo và thực thi

`crates/voice-agent-server/src/services/task_flow/mod.rs` khởi tạo WikiEngine, Orchestrator và TaskManager. TaskManager được prepare trước để cung cấp HostTool, sau đó bind Orchestrator cùng WikiAutoFlowBinding và bắt đầu chạy. TaskManager sở hữu `tasks.db` và `tasks.owner.lock`; công việc tiếp tục độc lập với Voice Session hay request Admin đã tạo nó.

- `daily_root` nhận công việc người dùng và điều phối Sub-Agent theo catalog/cấu hình đang có.
- `wiki_root` xử lý Wiki Auto-Flow khi có Markdown mới trong `sources/`.
- Voice dùng TaskToolSuite qua `services/task_flow/voice.rs`; Admin tạo và quản lý Task qua `app/admin/background/jobs.rs`.
- `/api/admin/jobs*` là API tương thích cho Background Task, không phải JobEngine cũ.

Cấu hình Agent và tool được resolve trong `services/task_flow/agent_settings/`. Danh mục Sub-Agent và skill có thể được Admin thay đổi, nên không dùng danh sách vai trò hoặc số lượng Agent cố định làm contract runtime. Xem [quản trị Sub-Agent](../subagent-catalog-and-web-management-implementation-guide.md) và [Job Skill Catalog](../job-skill-catalog-and-web-management-implementation-guide.md).

## Xuất bản và đọc artifact

HostTool tạo báo cáo Markdown, hình ảnh, video, âm thanh hoặc dữ liệu cấu trúc qua ArtifactStore của Orchestrator. Artifact gắn với cây Session của Task và mang metadata về Agent/tool tạo ra nó. Đây là cơ chế thay thế worker `/work/out` và bảng `job_artifacts` của Job V3 cũ.

1. Tool xuất bản bytes qua `ctx.artifacts().put(...)`; ArtifactStore quản lý staging, checksum, commit và metadata.
2. `TaskManager::get_task_detail_any_scope` cung cấp TaskDetailView và artifact thuộc Task.
3. `project_task_to_admin_job` trong `services/task_flow/projections.rs` xuất metadata tương thích: ID, tên, MIME, loại, vai trò, kích thước, SHA-256, producer Session/Agent/tool và `download_url`.
4. `GET /api/admin/jobs/{id}/artifacts/{artifact_id}` gọi `read_task_artifact_bytes_any_scope`. Handler hiện nằm ở `app/admin/background/artifacts.rs`; việc đọc phải qua TaskManager để kiểm tra quyền sở hữu và dữ liệu lưu trữ.
5. `GET /api/admin/artifacts` cung cấp catalog có lọc theo Task, flow, Agent, loại và vai trò artifact.

Không dùng tên file hiển thị làm storage key. Không tự đọc đường dẫn artifact do model trả về để bỏ qua TaskManager.

## Recovery, chờ và ngân sách

Trạng thái Task và Agent Run do runtime quyết định. Chính sách hiện hành nằm ở [ADR 0133](../adr/0133-durable-waiting-retry-and-job-budgets.md), [ADR 0134](../adr/0134-resumable-tool-operations-and-job-muse-sessions.md) và [hướng dẫn độ bền Job](../agent-job-durability-implementation-guide.md).

Retry/restart không đặt lại ngân sách của cùng một turn. Tool có tác dụng phụ chưa rõ kết quả không được chạy lại mù; tool Resumable phải đối soát từ journal. Task Follow-up là yêu cầu mới và có ngân sách mới. Run chờ được lưu bền, còn Task hết ngân sách ánh xạ sang `stopped` và giữ kết quả từng phần.

Các test local và gate provider thật có phạm vi khác nhau; trạng thái kiểm chứng xem [biên bản durability](../providers/agent-job-durability-validation-2026-10-31.md).
