# 06: Hiển thị tiến độ của Job recipe

**What to build:** Admin nhận ra Job recipe trong danh sách Job và theo dõi được recipe đang ở bước nào trên trang chi tiết: bước hiện tại, số mục đã xong của bước lặp, và bước dừng kèm lý do. Tiến độ do `progress` của recipe tính từ transcript, nên không có bảng tiến độ riêng.

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] TaskView và TaskDetailView có trường tùy chọn `recipe` chứa `id`, `version`, `params` và `RecipeProgress`: trạng thái từng bước trong outline, bước hiện tại, số mục đã xong trên tổng số với bước lặp, cùng `stop_reason` và bước lỗi nếu có.
- [ ] Projection Admin Job trả nguyên `RecipeProgress`, không diễn giải lại. Có test contract HTTP.
- [ ] Danh sách Job hiện nhãn `recipe · <tên> v<version>` dưới mục tiêu.
- [ ] Trang chi tiết Job có thanh tiến độ bước ở trên timeline hiện có, và cập nhật theo Job Change Signal như phần còn lại của trang.
- [ ] Agent Run và tool call của Root recipe và Agent Step vẫn hiện trong timeline.
- [ ] Có test hàm thuần `progress` cho recipe playlist, và Vitest cho thanh tiến độ ở các trạng thái đang chạy, hoàn tất và dừng.
