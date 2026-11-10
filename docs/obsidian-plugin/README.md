# Wiki Obsidian Plugin

Status: **v1 đã có code** (2026-11-10) ở `apps/wiki-obsidian`. Phần lõi có unit test; plugin chưa được chạy thử trong Obsidian.

Plugin Obsidian mang logic LLM Wiki (xem [../llm-wiki.md](../llm-wiki.md)) vào thẳng vault: chat hỏi đáp có trích dẫn, và agent đọc, viết, ingest, lint các file Markdown. Hướng này thay cho việc tự dựng app wiki ([../wiki-desktop/README.md](../wiki-desktop/README.md)): Obsidian đã là editor, cây file, file watcher và graph view.

## Quyết định chính

- **TypeScript thuần, không có Rust.** Agent loop chạy trong plugin và gọi thẳng API của LLM. Không có sidecar, không có WASM. Plugin chạy được trên mobile và đưa được lên Community Plugins.
- **Obsidian sở hữu vault.** Mọi thao tác đọc ghi đi qua `app.vault`, nên editor đang mở luôn đồng bộ. Đây là nguyên tắc một writer của ADR 0005 áp vào Obsidian.
- **Port phần có giá trị của `llm-wiki-rs`:** prompts, tên và schema của tool, quy trình ingest/query/lint, định dạng `index.md` và `log.md`. Không port fs, path watcher, SQLite, parser Markdown: Obsidian hoặc một index trong bộ nhớ thay thế.
- **Giữ nguyên tên tool (`wiki_get`, `wiki_query`, ...)** để prompts dùng lại được và vault do `llm-wiki-rs` tạo vẫn mở được bằng plugin.
- **LLM qua API tương thích OpenAI** (`POST {baseUrl}/chat/completions` có `tools`), giống `vela-server`. Gọi bằng `requestUrl` của Obsidian để tránh CORS và chạy được trên mobile. v1 chưa stream.
- **Ghi file phải được duyệt.** Mỗi lệnh ghi, sửa, xoá của agent mở hộp thoại có diff. Có thể bật tự duyệt trong phần cài đặt.

## Bố cục vault

Giống `llm-wiki-rs`, đặt trong thư mục gốc cấu hình được (mặc định là gốc vault):

```
<root>/
  SCHEMA.md          quy ước của wiki, agent đọc ở mọi lượt
  sources/           nguồn thô, agent chỉ đọc khi tổng hợp
  wiki/
    index.md         danh mục mọi trang
    log.md           nhật ký ingest / query / lint
    sources/         tóm tắt 1-1 theo tên file nguồn
    entities/  concepts/  syntheses/
```

## Sơ đồ module

```
src/main.ts ── Plugin: view, lệnh, cài đặt
 ├─ obsidian/chatView.ts      ItemView chat
 ├─ obsidian/confirmModal.ts  duyệt thao tác ghi, có diff
 ├─ obsidian/vaultPort.ts     adapter VaultPort trên app.vault
 ├─ obsidian/llmClient.ts     adapter LlmClient trên requestUrl
 └─ core/  (không import `obsidian`, test bằng vitest)
     ├─ agent.ts     vòng lặp tool-calling, giới hạn bước, huỷ
     ├─ tools.ts     định nghĩa tool và executor
     ├─ wiki.ts      WikiStore: index trong bộ nhớ, search BM25, link, ingest, synthesis
     ├─ lint.ts      nguồn chưa ingest, link gãy, trang mồ côi, thiếu frontmatter
     ├─ indexMd.ts / logMd.ts / render.ts / frontmatter.ts / paths.ts / diff.ts
     └─ prompts.ts   system prompt cho chat, agent, ingest; chỉ thị ngôn ngữ
```

Hai seam, mỗi seam có hai adapter (thật và giả trong test):

| Seam | Interface | Adapter thật | Adapter test |
|---|---|---|---|
| Vault | `VaultPort`: `list`, `read`, `write`, `remove`, `mtime` | `ObsidianVault` | `MemoryVault` |
| LLM | `LlmClient.chat(messages, tools, signal)` | `OpenAiCompatClient` | client kịch bản |

## Hành vi

**Chat.** Hai chế độ. *Hỏi* chỉ có tool đọc (`wiki_query`, `wiki_get`, `wiki_multi_get`, `wiki_graph_neighbors`, `wiki_tree`). *Agent* có thêm tool ghi. File đang mở có thể ghim làm ngữ cảnh. Câu trả lời render bằng Markdown của Obsidian nên `[[link]]` bấm được. Nút "Lưu vào wiki" ghi câu trả lời cuối vào `wiki/syntheses/`. Nút "Lưu hội thoại" ghi cả cuộc chat ra `chats/`.

**Ingest.** Lệnh *Ingest file hiện tại* chạy agent với prompt ingest 4 bước của `llm-wiki-rs` trên file trong `sources/`. `wiki_ingest_source` ghi trang tóm tắt cùng tên file, cập nhật `index.md` và `log.md`.

**Lint.** Lệnh *Lint wiki* chạy kiểm tra tất định (không gọi LLM) và đưa báo cáo vào chat để agent sửa tiếp nếu muốn.

**Giới hạn.** Số bước tool mỗi lượt (mặc định 12, ingest 30), kết quả tool cắt ở 12 000 ký tự, đường dẫn tool phải nằm trong `sources/`, `wiki/` hoặc là `SCHEMA.md`. Kết quả tool là dữ liệu, không phải chỉ dẫn (ghi trong system prompt).

## Khác với `llm-wiki-rs`

- Search là BM25 trong bộ nhớ, chưa có vector, RRF, rerank, filter metadata. `wiki_query` chỉ nhận `query`, `collections`, `limit`.
- Không ghi khối `qmd.metadata` vào frontmatter. Trang cũ có khối này vẫn đọc được.
- Không có `#docid`. Tài liệu được gọi bằng đường dẫn hoặc tên file.
- Chưa có `wiki_metadata`, `wiki_status`, subagent, ingest theo chunk cho nguồn dài.

## API key

Khoá lưu bằng `app.secretStorage` khi Obsidian hỗ trợ, nếu không thì nằm trong `data.json` của plugin. Trường hợp sau, **đừng commit `.obsidian/plugins/wiki-agent/data.json`** nếu vault được đồng bộ bằng Git.

## Chạy thử

```bash
cd apps/wiki-obsidian && npm install && npm run build
```

Plugin không nằm trong npm workspace ở gốc repo: thêm nó vào workspace làm npm 11 xoá binding native của rolldown khỏi `package-lock.json` chung. Nó có `package-lock.json` riêng và `.npmrc` bật `legacy-peer-deps`, vì npm 11 lỗi khi giải peer dependency của `vitest` và `obsidian` ở thư mục này.

Chép `main.js`, `manifest.json`, `styles.css` vào `<vault>/.obsidian/plugins/wiki-agent/`, bật plugin, điền Base URL, model và API key, rồi chạy lệnh *Wiki Agent: Khởi tạo cấu trúc wiki*.

Kiểm tra: `scripts/agent-test.sh run -- npm --prefix apps/wiki-obsidian test` và `... run typecheck`.

## Sau v1

Stream câu trả lời, mở lại hội thoại cũ, search vector, ingest nguồn dài theo chunk, lint ngữ nghĩa bằng LLM (mâu thuẫn, thông tin cũ), nhập PDF/Office.
