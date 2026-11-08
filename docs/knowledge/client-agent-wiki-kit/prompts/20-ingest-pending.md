# Prompt — Ingest các nguồn còn tồn (`uningested_sources`)

```text
Ingest mọi nguồn trong uningested_sources của wiki_lint, từng nguồn một.

Với mỗi nguồn:
1. Nạp skill `wiki-ingest`, `wiki-taxonomy`, `wiki-page-format`.
2. Đọc nguồn theo cửa sổ dòng (wiki_get from_line/max_lines) tới hết, không đọc trùng from_line.
3. Phân tích: domain/topic, 3 facet tag, entity/concept (Page-worthy yes/no), mâu thuẫn với trang hiện có.
4. wiki_ingest_source với source_rel_path đúng, title, one_line_summary, tags, related_entities, related_concepts, wiki_markdown_body.
5. Tạo hoặc gộp trang entity/concept Page-worthy=yes (wiki_write_file / wiki_patch_file append_text).
6. Vá link ngược từ các tóm tắt nguồn liên quan.
7. wiki_lint; nguồn này không được còn trong uningested_sources hay incomplete_ingests.

Dừng và hỏi tôi nếu: nguồn không đọc được, domain/topic không khớp folder nào hiện có và SCHEMA.md không hướng dẫn, hoặc nguồn mâu thuẫn mạnh với trang hiện có.
Cuối cùng báo: nguồn đã ingest, trang tạo/sửa, cảnh báo mâu thuẫn đã thêm.
```
