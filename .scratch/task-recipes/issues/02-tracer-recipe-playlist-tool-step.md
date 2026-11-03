# 02: Tracer: recipe playlist chỉ có Tool Step chạy hết từ TaskManager tới Admin API

**What to build:** Người vận hành gọi Admin API để xem danh sách recipe và tạo một Job từ recipe "thu thập video vào `sources/`" với domain cố định. Job chạy hết các Tool Step mà không gọi LLM, và tạo đúng file trong `sources/`. Đây là đường xuyên suốt đầu tiên: interface recipe, adapter `ProviderRuntime` xác định, Root recipe, validate tham số, ghim version, TaskManager, server và HTTP.

**Blocked by:** 01

**Status:** ready-for-agent

- [ ] Recipe có `id`, `version`, `title`, `description`, `params_schema`, `outline`, `max_steps`, cùng hai hàm thuần `next(params, transcript)` và `progress(params, transcript)`.
- [ ] Adapter khai báo alias `recipe`, đọc `<id>@<version>` từ `model_id` và tham số từ user message đầu tiên, gọi `next`, và luôn trả usage bằng 0.
- [ ] Mỗi recipe có Root agent riêng, với tool và Sub-Agent đúng theo outline, step cap bằng `max_steps`, và context limit đủ để compaction không tóm tắt mất tool result.
- [ ] Registry từ chối khởi động khi có recipe trùng `id`, outline tham chiếu HostTool hoặc Sub-Agent không tồn tại, hoặc `params_schema` ngoài tập con (`string` có `format: uri`, `integer`, `boolean`, `enum` có `x-options-source`, `array<string>`).
- [ ] TaskManager liệt kê được recipe kèm metadata và `outline`.
- [ ] `CreateTaskRequest` nhận `recipe { id, params }`. Tham số sai bị từ chối kèm `validation_path`, và không có Task nào được tạo. TaskManager ghim version hiện tại vào `model_id` của Root, và sinh tiêu đề khi request không truyền.
- [ ] Bước lặp phát nhiều tool call trong một bước, theo lô 10 mục. Danh sách mục nằm trong Collection Ledger hiện có. Mục lỗi bị cách ly, các mục khác vẫn xong.
- [ ] Recipe trả thất bại thì Task kết thúc với `RECIPE_STEP_FAILED`, giữ kết quả từng phần, không có lượt agent loop nào tiếp theo.
- [ ] Test TaskManager dùng `yt-dlp` giả cho thấy recipe chạy hết, file `sources/` đúng, ScriptedGateway không nhận lượt LLM nào, và token của lượt recipe bằng 0.
- [ ] `next` của recipe playlist có test hàm thuần với transcript dựng sẵn.
- [ ] `GET /api/admin/background/recipes` và `POST /api/admin/jobs` có `recipe` hoạt động. Lỗi tham số trả `validation_path` theo định dạng lỗi admin hiện có. Có test contract HTTP.
- [ ] Task không có recipe vẫn chạy agent loop như cũ. Test hiện có tiếp tục pass.
- [ ] Server dùng thư viện qua `[patch.crates-io]` trỏ tới nhánh feature, theo `AGENTS.md`.
