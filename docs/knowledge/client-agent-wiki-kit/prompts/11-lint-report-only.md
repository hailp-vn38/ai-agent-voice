# Prompt — Chỉ báo cáo lint (dry run, token read-only dùng được)

```text
Chạy wiki_lint và báo cáo, KHÔNG sửa gì.

1. Đọc SCHEMA.md và wiki/index.md.
2. Chạy wiki_lint.
3. Với mỗi loại lỗi, liệt kê tối đa 10 mục đầu kèm số tổng, và đề xuất cách sửa cụ thể (tool + tham số chính).
4. Quét ngữ nghĩa: cặp trang nghi trùng, claim có thể mâu thuẫn, khái niệm nên có trang.
5. Xếp ưu tiên: lỗi chặn is_healthy trước, sau đó lỗi chất lượng.

Đầu ra: một bảng Markdown (loại lỗi | số lượng | ví dụ | cách sửa) và danh sách 3–5 việc nên làm trước.
```
