# 06: Hiển thị tiến độ của Job recipe

**What to build:** Admin nhận ra Job recipe trong danh sách Job và theo dõi được recipe đang ở bước nào trên trang chi tiết: bước hiện tại, số mục đã xong của bước lặp, và bước dừng kèm lý do. Tiến độ do `progress` của recipe tính từ transcript, nên không có bảng tiến độ riêng.

**Blocked by:** 02

**Status:** resolved

- [x] TaskView và TaskDetailView có trường tùy chọn `recipe` chứa `id`, `version`, `params` và `RecipeProgress`: trạng thái từng bước trong outline, bước hiện tại, số mục đã xong trên tổng số với bước lặp, cùng `stop_reason` và bước lỗi nếu có.
- [x] Projection Admin Job trả nguyên `RecipeProgress`, không diễn giải lại. Có test contract HTTP.
- [x] Danh sách Job hiện nhãn `recipe · <tên> v<version>` dưới mục tiêu.
- [x] Trang chi tiết Job có thanh tiến độ bước ở trên timeline hiện có, và cập nhật theo Job Change Signal như phần còn lại của trang.
- [x] Agent Run và tool call của Root recipe và Agent Step vẫn hiện trong timeline.
- [x] Có test hàm thuần `progress` cho recipe playlist, và Vitest cho thanh tiến độ ở các trạng thái đang chạy, hoàn tất và dừng.

## Answer

Đã merge vào integration branch `feat/task-recipes` qua server `23cc0b2` và flow `4a3f3b3`. TaskView/TaskDetailView mang recipe metadata + `RecipeProgress` được dựng từ lịch sử Root; Admin projection giữ nguyên progress. Jobs list có nhãn recipe; Job detail hiển thị tiến độ trước timeline và tự cập nhật qua Job Change Signal. Test xác nhận progress không làm mất Root/Agent Step runs hay tool calls.

Kiểm tra: Admin Web 530/530 trên 91 file, typecheck, flow `scripts/check.sh check` và `scripts/check.sh test` pass; targeted server progress, HTTP projection và timeline tests pass. Server package suite vẫn fail ở `tests/vieneu.rs:66` vì thiếu fixture; flow `fast` vẫn dừng tại lint nền `src/tools/pdf.rs:142`, ngoài diff.
