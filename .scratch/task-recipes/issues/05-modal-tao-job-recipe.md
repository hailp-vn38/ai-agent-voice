# 05: Modal "Tạo job" chọn recipe và nhập tham số

**What to build:** Trên Admin Web, bấm "Tạo job" thì thấy ô tìm kiếm và lưới thẻ, với "Tự do" đứng đầu rồi tới các recipe. Chọn một recipe thì hiện form tham số dựng từ schema và danh sách bước chỉ đọc. Bấm "Chạy job" thì tạo Job và mở trang chi tiết. Thiết kế theo `docs/task-recipe-ui.md`.

**Blocked by:** 02

**Status:** resolved

- [x] Thẻ "Tự do" giữ nguyên hành vi hiện tại, gồm mục tiêu và đính kèm WAV.
- [x] Thẻ recipe hiện tên, mô tả và version. Ô tìm kiếm lọc được theo tên và mô tả.
- [x] Form render đúng control cho từng kiểu trong tập con: URL, number có min/max, checkbox, select (cả danh sách động) và danh sách nhiều dòng.
- [x] `default` được điền sẵn, và trường bắt buộc trống bị chặn phía client.
- [x] Lỗi server gắn vào đúng trường theo `validation_path`. Lỗi không khớp trường nào thì hiện ở đầu form.
- [x] Danh sách bước lấy từ `outline`, hiện loại Tool hay Agent và có lặp hay không.
- [x] Gửi lại cùng một lần tạo không tạo Job thứ hai, nhờ idempotency key hiện có.
- [x] Có Vitest cho form dựng từ schema và cho API client.

## Answer

Đã merge vào integration branch `feat/task-recipes` tại `a3086f2`. Đã thêm API/types catalog, thẻ recipe có tìm kiếm (Tự do đứng đầu), form schema-driven, domain Wiki động từ `/api/admin/knowledge/tree?space=wiki`, preview outline, validation/error mapping, idempotency và chuyển trang chi tiết; luồng goal cùng WAV/MP4 được giữ. Targeted Vitest 47/47, Admin Web suite 525/525 (90 files), typecheck và `git diff --check` pass. Chưa kiểm live server/API.
