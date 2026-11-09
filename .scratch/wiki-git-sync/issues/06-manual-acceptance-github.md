# 06: Nghiệm thu thủ công desktop với GitHub thật

Status: ready-for-human
Blocked by: 03, 05
Spec: ../spec.md (tất cả user stories)

## Mục tiêu

Xác nhận sync chạy đúng với repo GitHub private thật và credential thật trên macOS.

## Kịch bản

- [ ] Tạo repo private trống, cấu hình trong app bằng URL SSH; lần sync đầu push đủ `SCHEMA.md`, `sources/`, `wiki/`, không có `.wiki/`.
- [ ] Làm lại với URL HTTPS sau `gh auth setup-git`.
- [ ] Sửa trang trong app → khoảng 30 s sau có commit trên GitHub.
- [ ] Agent MCP ghi trang → commit có dòng `Origin: mcp`.
- [ ] Clone vault sang thư mục thứ hai, mở bằng cửa sổ khác hoặc máy khác, sửa cùng một trang ở hai nơi → có file conflict, không mất nội dung.
- [ ] Rút mạng rồi Sync now → lỗi `Network` rõ ràng, có mạng lại thì tự hết.
- [ ] Repo public → có cảnh báo.
- [ ] Mở `wiki-mcp` direct giữ lock → sync tạm tắt, footer báo lý do.
- [ ] Cập nhật `docs/wiki-desktop/git-sync.md` Status thành implemented.
