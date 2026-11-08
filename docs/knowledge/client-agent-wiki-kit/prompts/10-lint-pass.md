# Prompt — Lint toàn bộ wiki

```text
Chạy một lượt Wiki Lint đầy đủ trên vault qua MCP `knowledge`.

1. Nạp skill `wiki-lint`, `wiki-page-format`, `wiki-taxonomy` (và `wiki-ingest` nếu có nguồn chưa ingest).
2. Đọc SCHEMA.md, wiki/index.md, wiki_tree.
3. Chạy wiki_lint, ghi lại số lượng từng loại lỗi ban đầu.
4. Sửa theo đúng thứ tự trong skill: uningested_sources → stale_wiki_pages → incomplete_ingests → broken_links → orphan_wiki_pages → unlinked_mentions → missing_frontmatter_pages.
5. Làm kiểm tra ngữ nghĩa: trang trùng lặp, mâu thuẫn chưa gắn cảnh báo, khái niệm lặp lại chưa có trang.
6. Chạy lại wiki_lint, lặp tới khi is_healthy=true (tối đa 5 vòng).
7. wiki_log(mode="append", action="lint") ghi tóm tắt.

Ràng buộc: không sửa sources/, không sửa tay index.md/log.md, ưu tiên wiki_patch_file, không xóa trang nếu chưa gộp hết claim sang trang chuẩn.

Báo cáo cuối: bảng trước/sau từng loại lỗi; danh sách trang đã tạo/sửa/gộp/xóa; các việc cần tôi quyết định.
```
