# 11: Nghiệm thu thủ công server + Admin Web với GitHub thật

Status: ready-for-human
Blocked by: 09, 10
Spec: ../spec.md (user stories 11–16)

## Kịch bản

- [ ] Tạo repo private, thêm deploy key có quyền ghi, bật `[wiki.sync]`, khởi động lại stack prod → lần sync đầu push đủ `SCHEMA.md`, `sources/`, `wiki/`, không có `.wiki/`.
- [ ] Sửa trang trên Admin Web → sau debounce có commit trên GitHub; huy hiệu cập nhật không cần tải lại trang.
- [ ] Sửa trang trực tiếp trên GitHub → sau một chu kỳ (hoặc "Sync now") Admin Web hiện nội dung mới, tìm kiếm thấy.
- [ ] Sửa cùng một trang trên GitHub và Admin Web → có file conflict, panel liệt kê, không mất nội dung.
- [ ] Chạy một job AI ingest rồi bấm "Sync now" → trạng thái "Đang chờ job ingest", job xong thì sync.
- [ ] Sửa trang liên tục trong lúc đang sync → không lỗi, không mất nội dung.
- [ ] Xóa deploy key trên GitHub → lỗi `sync_auth` rõ ràng, không lộ đường dẫn khóa hay stderr thô.
- [ ] Panel trên Admin Web không có trường sửa remote.
- [ ] Cập nhật `docs/wiki-desktop/git-sync.md` Status thành implemented.
