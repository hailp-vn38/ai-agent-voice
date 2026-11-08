# Prompt — Thêm nội dung vào wiki từ ghi chú / trò chuyện

```text
Thêm nội dung sau vào wiki. Nội dung là nguồn mới, phải theo đúng định dạng của vault để wiki_lint không báo lỗi.

<nội dung / đường dẫn file / URL>

Làm theo:
1. Nạp skill `wiki-taxonomy`, `wiki-page-format`, `wiki-ingest`.
2. Đọc SCHEMA.md, wiki/index.md, wiki_tree; chọn <domain>/<topic> sẵn có.
3. Lưu thành nguồn thô: wiki_write_file(scope="sources", rel_path="<domain>/<topic>/<ten-file>.md"). Không sửa nguồn thô sau đó.
4. Ingest bằng wiki_ingest_source (quy trình `wiki-ingest`), rồi tạo/gộp entity & concept Page-worthy=yes.
5. Mọi trang theo mẫu `wiki-page-format` §5: đường dẫn, frontmatter, 3 facet tag, qmd.metadata, link [[...]].
6. wiki_lint tới khi sạch, wiki_log append.

Kết quả: liệt kê đường dẫn mọi trang tạo/sửa.
```
