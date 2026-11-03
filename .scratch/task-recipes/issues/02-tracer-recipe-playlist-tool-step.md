# 02: Tracer: recipe playlist chỉ có Tool Step chạy hết từ TaskManager tới Admin API

**What to build:** Người vận hành gọi Admin API để xem danh sách recipe và tạo một Job từ recipe "thu thập video vào `sources/`" với domain cố định. Job chạy hết các Tool Step mà không gọi LLM, và tạo đúng file trong `sources/`. Đây là đường xuyên suốt đầu tiên: interface recipe, adapter `ProviderRuntime` xác định, Root recipe, validate tham số, ghim version, TaskManager, server và HTTP.

**Blocked by:** 01

**Status:** resolved

- [x] Recipe có `id`, `version`, `title`, `description`, `params_schema`, `outline`, `max_steps`, cùng hai hàm thuần `next(params, transcript)` và `progress(params, transcript)`.
- [x] Adapter khai báo alias `recipe`, đọc `<id>@<version>` từ `model_id` và tham số từ user message đầu tiên, gọi `next`, và luôn trả usage bằng 0.
- [x] Mỗi recipe có Root agent riêng, với tool và Sub-Agent đúng theo outline, step cap bằng `max_steps`, và context limit đủ để compaction không tóm tắt mất tool result.
- [x] Registry từ chối khởi động khi có recipe trùng `id`, outline tham chiếu HostTool hoặc Sub-Agent không tồn tại, hoặc `params_schema` ngoài tập con (`string` có `format: uri`, `integer`, `boolean`, `enum` có `x-options-source`, `array<string>`).
- [x] TaskManager liệt kê được recipe kèm metadata và `outline`.
- [x] `CreateTaskRequest` nhận `recipe { id, params }`. Tham số sai bị từ chối kèm `validation_path`, và không có Task nào được tạo. TaskManager ghim version hiện tại vào `model_id` của Root, và sinh tiêu đề khi request không truyền.
- [x] Bước lặp phát nhiều tool call trong một bước, theo lô 10 mục. Danh sách mục nằm trong Collection Ledger hiện có. Mục lỗi bị cách ly, các mục khác vẫn xong.
- [x] Recipe trả thất bại thì Task kết thúc với `RECIPE_STEP_FAILED`, giữ kết quả từng phần, không có lượt agent loop nào tiếp theo.
- [x] Test TaskManager dùng `yt-dlp` giả cho thấy recipe chạy hết, file `sources/` đúng, ScriptedGateway không nhận lượt LLM nào, và token của lượt recipe bằng 0.
- [x] `next` của recipe playlist có test hàm thuần với transcript dựng sẵn.
- [x] `GET /api/admin/background/recipes` và `POST /api/admin/jobs` có `recipe` hoạt động. Lỗi tham số trả `validation_path` theo định dạng lỗi admin hiện có. Có test contract HTTP.
- [x] Task không có recipe vẫn chạy agent loop như cũ. Test hiện có tiếp tục pass.
- [x] Server dùng thư viện qua `[patch.crates-io]` trỏ tới nhánh feature, theo `AGENTS.md`.

## Answer

Đã merge vào integration branch `feat/task-recipes` qua các merge commits server `c53cc9c`, flow `0562a3b` và agent `34d01e2` (ticket 01). Recipe có ba bước riêng để liệt kê/đọc ledger, fetch transcript và lưu nguồn theo batch 10; item fetch lỗi được thử tối đa ba lần rồi chuyển `gave_up`, không chặn video khác. Registry kiểm tra reference HostTool/Sub-Agent khi khởi động. HTTP test với fake `yt-dlp` xác nhận recipe hoàn tất, `sources/` đúng, usage 0, `validation_path`, lỗi `RECIPE_STEP_FAILED` và Task tự do vẫn chạy; flow registry/package tests và server check/clippy pass. Flow `scripts/check.sh fast` còn cảnh báo nền `src/tools/pdf.rs:142` (`too_many_arguments`), ngoài diff.

## Review follow-up — 2026-11-03

Recipe nhận đúng một trong hai: URL playlist hoặc tối đa 50 URL video YouTube. Danh sách video đi qua cùng `yt_list_playlist`, Collection Ledger, transcript và `add_to_sources` HostTool như playlist. Checkbox Wiki mặc định tắt: TaskStore ghi marker hoãn theo `video_id` trước khi nguồn xuất hiện; Auto-Flow loại trừ đúng các nguồn đã đánh dấu. Yêu cầu ingest tường minh xóa marker theo đường dẫn sau khi TaskManager nhận Task. Tập trung test HTTP xác nhận hai video lẻ được lưu và cờ bật/tắt được truyền đúng; test Auto-Flow xác nhận nguồn tắt bị bỏ qua, nguồn thường vẫn chạy và nguồn đã resume được xử lý.
