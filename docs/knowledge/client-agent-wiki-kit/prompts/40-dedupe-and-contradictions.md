# Prompt — Khử trùng lặp và đánh dấu mâu thuẫn

```text
Rà soát ngữ nghĩa wiki cho chủ đề/domain: <domain hoặc "toàn bộ">.

1. Nạp skill `wiki-lint`, `wiki-page-format`.
2. Từ wiki/index.md, wiki_tree và wiki_query, tìm các cặp trang entity/concept cùng chủ đề khác slug (kể cả alias, viết tắt, tiếng Anh/Việt).
3. Mỗi cặp: chọn trang chuẩn, gộp mọi claim, code, sources, aliases, tags (top-level và qmd.metadata) vào đó, vá mọi link đến trang trùng bằng wiki_patch_file, rồi wiki_delete_file trang trùng. Liệt kê cặp trước khi xóa nếu số cặp > 3 và chờ tôi xác nhận.
4. Tìm claim mâu thuẫn giữa các nguồn trong cùng chủ đề; thêm callout `> [!WARNING] Contradiction / Caveat` trích cả hai [[sources/...]]. Không tự chọn bên đúng.
5. wiki_lint tới khi sạch, wiki_log append.
```
