# 02: Tracer: recipe playlist chỉ có Tool Step chạy hết từ TaskManager tới Admin API

**What to build:** Người vận hành gọi Admin API để xem danh sách recipe và tạo một Job từ recipe "thu thập video vào `sources/`" với domain cố định. Job chạy hết các Tool Step mà không gọi LLM, và tạo đúng file trong `sources/`. Đây là đường xuyên suốt đầu tiên: registry recipe, validate tham số, ghim version, con trỏ bước lưu bền, TaskManager, server và HTTP.

**Blocked by:** 01

**Status:** ready-for-agent

- [ ] Host đăng ký recipe lúc khởi động. Registry từ chối khởi động khi có recipe trùng `(id, version)`, bước tham chiếu HostTool không tồn tại, hoặc `params_schema` ngoài tập con cho phép.
- [ ] TaskManager liệt kê được recipe kèm `id`, `version`, `title`, `description`, `params_schema` và danh sách bước rút gọn (`title`, `kind`, `repeat`).
- [ ] `CreateTaskRequest` nhận `recipe { id, version?, params }`. Tham số sai bị từ chối kèm `validation_path`, và không có Task nào được tạo. Không truyền version thì dùng bản mới nhất. Tiêu đề được sinh từ recipe khi request không truyền.
- [ ] Task lưu `recipe_id`, `version` và tham số đã validate. Từng bước được commit hoàn tất trong `tasks.db` trước khi sang bước sau.
- [ ] Tool Step gọi HostTool qua cùng đường dispatch với tool call của agent: `ToolEffect`, Tool Operation, quyền và ArtifactStore giữ nguyên.
- [ ] Bước lặp theo lô xử lý các mục trong Collection Ledger. Mục lỗi bị cách ly, các mục khác vẫn xong.
- [ ] Test TaskManager dùng `yt-dlp` giả cho thấy recipe chạy hết, file `sources/` đúng, và ScriptedGateway không nhận lượt LLM nào.
- [ ] Bước lỗi vĩnh viễn làm Task kết thúc kèm `stop_reason` và bước lỗi, giữ kết quả từng phần, không có lượt agent loop nào tiếp theo.
- [ ] `GET /api/admin/background/recipes` và `POST /api/admin/jobs` có `recipe` hoạt động. Lỗi tham số trả `validation_path` theo định dạng lỗi admin hiện có. Có test contract HTTP.
- [ ] Task không có recipe vẫn chạy agent loop như cũ. Test hiện có tiếp tục pass.
- [ ] Server dùng thư viện qua `[patch.crates-io]` trỏ tới nhánh feature, theo `AGENTS.md`.
