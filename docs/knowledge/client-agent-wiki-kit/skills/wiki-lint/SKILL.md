---
name: wiki-lint
description: "Chạy wiki_lint qua MCP, sửa từng loại lỗi (uningested_sources, incomplete_ingests, broken_links, orphan_wiki_pages, stale_wiki_pages, unlinked_mentions, missing_frontmatter_pages), khử trùng lặp, đánh dấu mâu thuẫn, rồi lint lại tới is_healthy=true. Dùng định kỳ hoặc sau batch ingest."
---

# Wiki Lint (client agent)

Skill này chạy trên client ngoài (Codex, OpenCode, Claude Code...) qua MCP `knowledge`. Cần token **read-write**; token read-only chỉ báo cáo, không sửa được.

Phụ thuộc: `wiki-page-format` (hình dạng trang), `wiki-taxonomy` (có đáng tạo trang không), `wiki-ingest` (khi phải ingest nguồn).

## 0. Chuẩn bị (bắt buộc, 1 lần mỗi phiên)

1. `wiki_schema(mode="read")` — `SCHEMA.md` thắng mọi quy ước khác.
2. `wiki_get(file="wiki/index.md")` và `wiki_tree(scope="wiki", include_metadata=true)`.
3. Nếu thiếu tool `wiki_write_file` / `wiki_patch_file` trong danh sách tool: token là read-only → chỉ chạy bước 1 và báo cáo, dừng.

## 1. Chạy lint

Gọi `wiki_lint`. Đọc các trường:

| Trường | Ý nghĩa | Ảnh hưởng `is_healthy` |
|---|---|---|
| `uningested_sources` | file trong `sources/` chưa có trang tóm tắt | có |
| `broken_links` | `[[...]]` trỏ tới trang không tồn tại | có |
| `orphan_wiki_pages` | trang không có link đến | có |
| `stale_wiki_pages` | nguồn thô mới hơn trang wiki | có |
| `incomplete_ingests` | tóm tắt nguồn đã có nhưng entity/concept nó nhắc chưa được tạo, hoặc nguồn đổi sau khi ingest | không (nhưng phải xử lý) |
| `unlinked_mentions` | nhắc tên entity/concept nhưng chưa bọc `[[ ]]` | không |
| `missing_frontmatter_pages` | trang thiếu YAML frontmatter | không |
| `flagged_contradictions_or_gaps` | marker `CONTRADICTION:`, `[!WARNING]`, `TODO:`, `GAP:` | không |

Xử lý theo thứ tự dưới đây (cái trước tạo ra link/trang cho cái sau). Ghi lại số lượng ban đầu để báo cáo.

## 2. Sửa theo thứ tự

1. **`uningested_sources`** → đọc nguồn rồi ingest bằng `wiki-ingest` (`wiki_ingest_source`). Mỗi lần một nguồn, lint lại sau mỗi nguồn nếu danh sách dài.
2. **`stale_wiki_pages`** → đọc lại nguồn thô, so với trang; cập nhật claim đã đổi bằng `wiki_patch_file` hoặc ingest lại. Không xóa claim cũ nếu nguồn cũ vẫn tồn tại; nếu hai bản mâu thuẫn, xem mục 3.
3. **`incomplete_ingests`** → với mỗi `missing_targets`: nếu `Page-worthy: yes` (xem `wiki-taxonomy` §3) thì tạo trang bằng `wiki_write_file` theo `wiki-page-format`; nếu không đáng thì sửa link trong trang tóm tắt thành văn bản thường.
4. **`broken_links`** → hai lựa chọn, chọn theo `Page-worthy`:
   - đáng có trang → `wiki_write_file` tạo trang đích;
   - không đáng, hoặc gõ sai slug → `wiki_patch_file(target_text, replacement_text)` sửa slug/bỏ `[[ ]]`.
   Trước khi tạo, tìm slug đã tồn tại (`wiki/index.md`, `wiki_query`) để khỏi tạo trùng.
5. **`orphan_wiki_pages`** → thêm link đến từ trang liên quan (`wiki/sources/`, `wiki/entities/`, `wiki/syntheses/`) bằng `wiki_patch_file(append_text=...)`, hoặc gộp/xóa trang thừa (mục 3). Không tạo link vô nghĩa chỉ để hết orphan.
6. **`unlinked_mentions`** → `wiki_patch_file`: bọc **lần nhắc đầu tiên** thành `[[entities/<domain>/<slug>|Thuật ngữ]]` hoặc `[[concepts/<domain>/<slug>|Thuật ngữ]]`. Không đụng frontmatter, code block, bảng ngoài link.
7. **`missing_frontmatter_pages`** → thêm đủ `title`, `category`, `summary`, `tags` (entity/concept thêm `domain`, `topic`, `aliases`, `sources`, `qmd`) theo mẫu trong `wiki-page-format` §5. Giữ nguyên nội dung thân bài.

Quy tắc bảo vệ:
- Không sửa `wiki/index.md`, `wiki/log.md` bằng tay (công cụ tự quản).
- Không sửa file trong `sources/` (bất biến).
- Link nội bộ luôn là `[[...]]` trần, ngoài backtick; trong ô bảng escape `\|`.
- Mỗi lần chỉ sửa tối thiểu; dùng `wiki_patch_file` thay vì ghi đè cả trang khi có thể.

## 3. Kiểm tra ngữ nghĩa (lint "mềm", không có trong `wiki_lint`)

1. **Trùng lặp**: trong `wiki/index.md` / `wiki_tree`, tìm hai trang cùng chủ đề khác slug (ví dụ `fvg` và `fair-value-gap`). Gộp mọi claim, code, `sources`, `aliases`, `tags` vào trang chuẩn (`wiki_write_file`), vá link đến trang trùng (`wiki_patch_file`), rồi `wiki_delete_file` trang trùng.
2. **Mâu thuẫn / claim cũ**: nếu hai nguồn bất đồng, giữ cả hai quan điểm và thêm
   ```markdown
   > [!WARNING] Contradiction / Caveat
   > Nguồn A nói ...; nguồn B nói ... — [[sources/...|A]] vs [[sources/...|B]].
   ```
3. **Khái niệm xuất hiện nhiều nhưng chưa có trang**: nếu qua ≥2 nguồn và có claim thực chất → tạo trang. Nếu không chắc, chỉ liệt kê trong báo cáo, đừng tạo.
4. **Khoảng trống dữ liệu**: liệt kê câu hỏi nên điều tra / nguồn nên tìm thêm trong báo cáo; không tự bịa nội dung.

## 4. Xác nhận sạch

1. Chạy lại `wiki_lint`. Lặp mục 2 cho tới khi: `uningested_sources`, `broken_links`, `orphan_wiki_pages`, `stale_wiki_pages` đều rỗng và `is_healthy: true`. Dừng ở tối đa 5 vòng; nếu còn lỗi, báo cáo phần còn lại và lý do.
2. Ghi nhật ký: `wiki_log(mode="append", action="lint", ...)` với tóm tắt ngắn (số lỗi trước/sau, trang đã tạo/gộp/xóa).
3. Báo cáo cho người dùng: bảng trước/sau, danh sách trang đã đổi, việc cần quyết định của con người (xóa trang, mâu thuẫn chưa giải quyết, nguồn nên bổ sung).
