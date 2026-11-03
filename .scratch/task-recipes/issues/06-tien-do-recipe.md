# 06: Hiển thị tiến độ của Job recipe

**What to build:** Admin nhận ra Job recipe trong danh sách Job và theo dõi được recipe đang ở bước nào trên trang chi tiết: bước hiện tại, số mục đã xong của bước lặp, và bước dừng kèm lý do.

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] TaskView và TaskDetailView có recipe đã ghim cùng trạng thái từng bước, số mục đã xong trên tổng số với bước lặp, và bước dừng kèm `stop_reason`.
- [ ] Projection Admin Job có các trường trên. Có test contract HTTP.
- [ ] Danh sách Job hiện nhãn `recipe · <tên> v<version>` dưới mục tiêu.
- [ ] Trang chi tiết Job có thanh tiến độ bước ở trên timeline hiện có, và cập nhật theo Job Change Signal như phần còn lại của trang.
- [ ] Agent Run và tool call của Agent Step vẫn hiện trong timeline.
- [ ] Có Vitest cho thanh tiến độ ở các trạng thái đang chạy, hoàn tất và dừng.
