# Client Agent Wiki Kit

Bộ prompt + skill để một client agent bên ngoài (Codex, OpenCode, Claude Code...) hiểu định dạng LLM Wiki của dự án, chạy **Wiki Lint** và thêm nội dung vào wiki qua MCP `knowledge` (`/mcp/knowledge`).

## 1. Agent cần biết gì

| Cần biết | Nằm ở đâu | Nếu thiếu thì |
|---|---|---|
| Cấu trúc 3 lớp, đường dẫn, slug, thế nào là trang đáng tạo | skill `wiki-taxonomy` | Tạo trang rác, trùng slug, sai folder |
| Hình dạng trang: frontmatter, 3 facet tag, `qmd.metadata`, `[[link]]`, bảng, Mermaid | skill `wiki-page-format` | Trang bị `missing_frontmatter_pages`, `broken_links`, link chết trong backtick |
| Quy trình nạp nguồn (4 giai đoạn) | skill `wiki-ingest` | `uningested_sources` / `incomplete_ingests` không hết |
| Quy trình lint + sửa từng loại lỗi + kiểm tra ngữ nghĩa | skill `wiki-lint` | Không sửa được tới `is_healthy: true` |
| Truy vấn, lưu tổng hợp (synthesis) | skill `wiki-query` | Synthesis mồ côi |
| Vai trò, quy tắc bắt buộc, cách chọn skill | `prompts/00-system-wiki-maintainer.md` | Agent hành xử như chatbot chung |
| Quy ước riêng của vault | `SCHEMA.md` (đọc qua `wiki_schema`) | Thắng mọi skill khi mâu thuẫn |

## 2. Danh sách file

```
client-agent-wiki-kit/
├── README.md
├── install.sh                         cài kit vào thư mục làm việc của client
├── prompts/
│   ├── 00-system-wiki-maintainer.md   prompt hệ thống (dán vào AGENTS.md)
│   ├── 10-lint-pass.md                lint + sửa đầy đủ
│   ├── 11-lint-report-only.md         chỉ báo cáo (token read-only dùng được)
│   ├── 20-ingest-pending.md           ingest các nguồn còn tồn
│   ├── 30-add-page-from-note.md       thêm nội dung mới vào wiki
│   └── 40-dedupe-and-contradictions.md  gộp trùng, gắn cảnh báo mâu thuẫn
├── skills/
│   ├── wiki-lint/SKILL.md             viết mới cho client (xem mục 5)
│   ├── wiki-page-format/SKILL.md      bản sao từ skills/ của repo
│   ├── wiki-taxonomy/SKILL.md         bản sao
│   ├── wiki-ingest/SKILL.md           bản sao
│   ├── wiki-ingest-files/SKILL.md     ingest bằng đọc/ghi file, không cần MCP (xem mục 7)
│   └── wiki-query/SKILL.md            bản sao
└── clients/
    ├── codex.config.toml              khai báo MCP cho Codex
    └── opencode.json                  khai báo MCP cho OpenCode
```

## 3. Cài đặt

**Bước 1 — bật MCP trên server** (`config.toml`):

```toml
[wiki]
enabled = true

[wiki.mcp]
enabled = true
path = "/mcp/knowledge"
allow_write = true       # false => mọi token chỉ đọc
```

Tạo token trong Admin (Knowledge → MCP). Token **read-write** mới thấy `wiki_write_file`, `wiki_patch_file`, `wiki_ingest_source`, `wiki_delete_file`...; token read-only chỉ dùng được `11-lint-report-only.md`. Server yêu cầu `Authorization: Bearer <token>` hoặc `x-api-key`.

**Bước 2 — khai báo MCP cho client:**

```bash
export WIKI_MCP_TOKEN=<token>
```

- Codex: thêm `clients/codex.config.toml` vào `~/.codex/config.toml`.
- OpenCode: gộp `clients/opencode.json` vào `opencode.json`.
- Claude Code: `claude mcp add --transport http knowledge http://HOST:PORT/mcp/knowledge --header "Authorization: Bearer $WIKI_MCP_TOKEN"`.

Sửa `127.0.0.1:8080` theo địa chỉ thật của server. Tên cấu hình có thể khác giữa các phiên bản client; đối chiếu với tài liệu của client nếu lỗi.

**Bước 3 — cài prompt + skill:**

```bash
docs/knowledge/client-agent-wiki-kit/install.sh /đường/dẫn/thư-mục-làm-việc all
```

Script: thêm prompt hệ thống vào `AGENTS.md` (không ghi đè), chép skill vào `.codex/skills`, `.opencode/skills`, `.claude/skills` (tùy tham số `codex|opencode|claude|all`) và một bản đọc được ở `wiki-kit/`. Client không có skill loader vẫn dùng được vì prompt hệ thống bảo agent đọc `SKILL.md` bằng tool đọc file. Nếu client của bạn tìm skill ở thư mục khác, chép thủ công.

## 4. Dùng

1. Mở client trong thư mục đã cài, kiểm tra tool `wiki_*` hiện ra.
2. Dán prompt trong `prompts/` (hoặc nói "chạy lint wiki").
3. Luồng điển hình: `11` xem tình trạng → `10` sửa → `20` nếu còn nguồn chưa ingest.
4. Nên chạy `10` sau mỗi batch ingest và định kỳ (ví dụ hàng tuần).

## 5. Khác biệt so với `skills/wiki-lint` trong repo

Skill của server (`skills/wiki-lint`, bản dùng cho agent nội bộ) chưa nói tới các trường mà `llm-wiki-rs` 0.2.x trả về: `stale_wiki_pages` (**chặn** `is_healthy`), `incomplete_ingests`, `flagged_contradictions_or_gaps`. Bản trong kit bổ sung các trường này, thêm bước phân biệt token read-only, thứ tự sửa và giới hạn vòng lặp. Bốn skill còn lại là bản sao nguyên văn; nguồn chuẩn vẫn là `skills/` của repo, nên khi sửa ở đó hãy chép lại vào kit.

## 6. Kiểm tra nhanh sau khi cài

- Hỏi agent: "liệt kê tool wiki bạn thấy" → phải có `wiki_lint`, `wiki_get`, `wiki_tree`; với token ghi có thêm `wiki_write_file`, `wiki_patch_file`.
- Chạy `11-lint-report-only.md`: phải trả bảng theo đúng các trường lint.
- Nhờ thêm một ghi chú nhỏ bằng `30-add-page-from-note.md`, rồi chạy `wiki_lint`: trang mới không được xuất hiện ở `broken_links`, `orphan_wiki_pages` hay `missing_frontmatter_pages`.

Chưa chạy thử end-to-end với Codex/OpenCode thật; cấu hình client ở trên viết theo định dạng thông dụng của từng công cụ.

## 7. Ingest không dùng MCP (`wiki-ingest-files`)

Khi client làm việc thẳng trên thư mục vault (bản git clone của vault, hoặc `data/wiki-vault`) và không có tool `wiki_*`, dùng skill `wiki-ingest-files`. Skill tự chứa: mẫu frontmatter, dạng link, cách thêm dòng vào `wiki/index.md` / `wiki/log.md` và lệnh shell tự kiểm tra, nên không cần nạp kèm `wiki-page-format` hay `wiki-taxonomy` (hai skill đó gọi tool MCP).

- Mở client tại gốc vault (thư mục có `SCHEMA.md`, `sources/`, `wiki/`) và nói "wiki-ingest các nguồn còn tồn" hoặc nêu file nguồn cụ thể.
- Prompt hệ thống `00-system-wiki-maintainer.md` bắt buộc dùng MCP; với luồng này đừng cài prompt đó, chỉ chép thư mục `skills/wiki-ingest-files/`.
- Server tự dựng lại `wiki/index.md` và index tìm kiếm từ file khi đồng bộ, nên trạng thái ingest chỉ phụ thuộc frontmatter và `[[link]]` mà agent ghi.
