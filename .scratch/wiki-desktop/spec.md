# Wiki Desktop: app macOS cho kho tri thức, có MCP

Status: ready-for-agent

Thiết kế chi tiết: `docs/wiki-desktop/` (README, core, mcp, desktop-shell, ui-reuse, implementation-plan).

## Problem Statement

Người dùng chỉ đọc và viết được Wiki Page cùng Raw Working Note qua trang Knowledge của Admin Web. Muốn vào trang này thì phải chạy `vela-server` (thường qua Docker) và mở trình duyệt. Vì vậy không dùng được kho tri thức như một app ghi chú cá nhân trên Mac: không chạy offline, không có cửa sổ native, không có phím tắt. Agent bên ngoài (Claude Desktop/Code, Cursor) chỉ đọc hoặc ghi wiki được khi server đang chạy và đã được cấu hình token. Khi agent ghi vào wiki, người dùng cũng không thấy thay đổi ngay trên giao diện.

Còn một vấn đề nữa: quy trình ghi một tài liệu (ghi file → đồng bộ Search Projection → cập nhật `index.md` → ghi log → báo UI) hiện bị rải giữa các route admin và tool MCP. Muốn có thêm một nơi host thứ hai thì quy trình này sẽ phải chép lại thêm một lần.

## Solution

Một app macOS độc lập tên **Wiki** (Tauri 2), mở một thư mục kho tri thức trên máy và hiển thị UI giống trang Knowledge (cây Raw/Wiki, đọc, soạn, tìm kiếm, Graph View, Backlink). Lõi wiki (`llm-wiki-rs`) chạy ngay trong app nên không cần server.

App có MCP server riêng với cùng bộ tool `wiki_*` mà `vela-server` đang có:
- Agent kết nối qua HTTP trên loopback khi app đang mở.
- Agent cũng có thể dùng binary stdio đi kèm. Binary này chuyển tiếp request sang app nếu app đang chạy, còn không thì tự mở kho tri thức.

Mọi thay đổi, dù do người dùng, agent hay ứng dụng khác như Obsidian/Git tạo ra, đều hiện ngay trên UI.

Để server và app không lệch nhau, phần lõi ghi/đọc và phần MCP được tách thành hai module dùng chung. Code UI Knowledge được tách thành một package dùng chung cho Admin Web và app.

## User Stories

### Mở và quản lý kho tri thức
1. As a người dùng Mac, I want cài app Wiki từ một file DMG đã ký và notarize, so that macOS cho chạy mà không cảnh báo.
2. As a người dùng, I want chọn một thư mục làm kho tri thức trong lần mở đầu tiên, so that tôi tự quyết định dữ liệu nằm ở đâu.
3. As a người dùng, I want app tạo cấu trúc chuẩn (`sources/`, `wiki/`) khi thư mục còn trống, so that tôi bắt đầu được ngay.
4. As a người dùng, I want mở một kho tri thức đang được `vela-server` dùng, so that tôi làm việc với dữ liệu sẵn có.
5. As a người dùng, I want app nhớ danh sách kho tri thức gần đây và mở lại kho cuối cùng, so that không phải chọn lại mỗi lần.
6. As a người dùng, I want chuyển kho tri thức bằng `⌘O`, so that tôi tách được wiki cá nhân và wiki công việc.
7. As a người dùng, I want app tự đồng bộ Search Projection khi mở một kho có file mới hơn index, so that kết quả tìm kiếm luôn đúng.
8. As a người dùng, I want bấm nút "Rebuild index", so that tôi tự sửa được khi index bị hỏng.
9. As a người dùng, I want thấy trạng thái kho (số Wiki Page, số Source, lần đồng bộ cuối), so that tôi biết kho đang ổn.

### Duyệt và đọc
10. As a người dùng, I want duyệt cây thư mục theo Knowledge Browser Space `raw` và `wiki` giống Admin Web, so that không phải học giao diện mới.
11. As a người dùng, I want đọc Wiki Page đã render Markdown, Mermaid và chart, so that nội dung hiển thị giống trên web.
12. As a người dùng, I want bấm vào Wiki Link để mở trang đích, so that tôi đi theo mạch kiến thức.
13. As a người dùng, I want thấy Backlink và các liên kết chưa phân giải của trang đang mở, so that tôi biết trang nằm ở đâu trong mạng liên kết.
14. As a người dùng, I want xem Graph View toàn kho và theo vùng lân cận, so that tôi nhìn được cấu trúc tri thức.
15. As a người dùng, I want đọc Source mà một Wiki Page trích dẫn (Wiki Provenance), so that tôi kiểm chứng được nội dung.
16. As a người dùng, I want mở nhiều cửa sổ (`⌘⇧N`), so that tôi đọc hai trang cạnh nhau.
17. As a người dùng, I want app theo chế độ sáng/tối của macOS và có sidebar vibrancy, so that app trông như app Mac native.

### Tìm kiếm
18. As a người dùng, I want mở nhanh trang theo tên bằng `⌘P`, so that tôi nhảy tới trang cần tìm ngay.
19. As a người dùng, I want tìm toàn văn bằng `⌘⇧F`, có đoạn trích và số dòng, so that tôi thấy ngữ cảnh trước khi mở.
20. As a người dùng, I want tìm tiếng Việt không dấu vẫn ra kết quả có dấu, so that gõ nhanh vẫn tìm được.
21. As a người dùng, I want lọc tìm kiếm theo `raw` hoặc `wiki`, so that kết quả gọn hơn.

### Soạn và sắp xếp
22. As a người dùng, I want tạo Wiki Page mới bằng `⌘N`, so that tôi ghi lại ý tưởng ngay.
23. As a người dùng, I want sửa trang ở chế độ Compose và lưu, so that tôi cập nhật kiến thức.
24. As a người dùng, I want được cảnh báo xung đột khi trang đã đổi kể từ lúc tôi mở (Knowledge Revision khác), so that tôi không ghi đè mất thay đổi của agent hay của cửa sổ khác.
25. As a người dùng, I want chọn giữ bản của tôi hoặc tải bản mới khi có xung đột, so that tôi tự quyết định.
26. As a người dùng, I want di chuyển hoặc đổi tên trang và thư mục bằng kéo thả, so that tôi sắp xếp kho tri thức.
27. As a người dùng, I want bị chặn hoặc được cảnh báo khi di chuyển làm gãy Wiki Link, so that không sinh ra liên kết hỏng.
28. As a người dùng, I want xóa trang (có xác nhận), so that tôi dọn kho.
29. As a người dùng, I want import nhiều file Markdown vào `raw` cùng lúc, so that tôi đưa tài liệu có sẵn vào nhanh.
30. As a người dùng, I want biên dịch một Raw Working Note thành Wiki Page qua `wiki_ingest_source` (không dùng LLM), so that tôi chuyển ghi chú thô thành tri thức có cấu trúc.

### Đồng bộ với thay đổi bên ngoài
31. As a người dùng dùng cả Obsidian, I want sửa file bên ngoài thì app tự cập nhật cây, trang và index, so that hai công cụ dùng chung một kho.
32. As a người dùng đồng bộ kho bằng Git hoặc iCloud, I want app nhận thay đổi sau khi pull hoặc sync, so that không phải khởi động lại.
33. As a người dùng, I want các lần ghi của chính app không bị báo lặp thành "thay đổi bên ngoài", so that UI không nhấp nháy.

### MCP cho agent
34. As a người dùng Claude Desktop, I want thêm app làm MCP server bằng một đoạn cấu hình trỏ tới binary stdio trong app, so that Claude đọc được wiki của tôi.
35. As a người dùng, I want nút "Cài vào Claude Desktop" ghi cấu hình sau khi tôi xác nhận, so that không phải tự sửa JSON.
36. As a người dùng Cursor hoặc Vela, I want copy URL HTTP MCP kèm token, so that kết nối được qua streamable HTTP.
37. As an agent, I want gọi các tool `wiki_tree`, `wiki_query`, `wiki_get`, `wiki_multi_get`, `wiki_metadata`, `wiki_status`, `wiki_graph_neighbors`, so that tôi khám phá và đọc kho tri thức.
38. As an agent có quyền ghi, I want gọi `wiki_write_file`, `wiki_patch_file`, `wiki_file_synthesis`, `wiki_ingest_source`, so that tôi bổ sung tri thức.
39. As a người dùng, I want tắt quyền ghi MCP, so that agent chỉ đọc được.
40. As a người dùng, I want có token riêng cho đọc và cho ghi, và đổi (rotate) được token, so that tôi kiểm soát quyền truy cập.
41. As a người dùng, I want thấy toast "Agent đã cập nhật <trang>" trong vòng một giây sau khi agent ghi, so that tôi biết agent vừa làm gì.
42. As a người dùng, I want footer hiện số client MCP đang kết nối và lần gọi gần nhất, so that tôi biết MCP đang hoạt động.
43. As a người dùng, I want binary stdio vẫn chạy được khi app đã đóng, so that Claude Code dùng được wiki mà không cần mở app.
44. As a người dùng, I want khi app đóng và binary stdio đang ghi thì mở app lên vẫn an toàn (app ở chế độ chỉ đọc kèm banner, tự lấy lại quyền ghi sau đó), so that index không bao giờ hỏng.
45. As a người dùng, I want các prompt và skill trong `client-agent-wiki-kit` dùng được nguyên vẹn với app, so that không phải viết lại.
46. As a người dùng, I want MCP từ chối request thiếu token hoặc có `Origin`/`Host` lạ, so that trang web độc hại không đọc được wiki qua localhost.
47. As an agent, I want nhận lỗi có kiểu (`Denied`, `InvalidArguments`, `ReadOnly`, `Timeout`, …), so that tôi xử lý được thay vì đoán.
48. As an agent, I want kết quả quá dài bị cắt theo giới hạn cấu hình, so that không tràn context.

### Admin Web và server (không được hỏng)
49. As an Admin dùng Admin Web, I want trang Knowledge giữ nguyên giao diện và hành vi sau khi tách code dùng chung, so that tôi không bị ảnh hưởng.
50. As an operator, I want `/mcp/knowledge` của `vela-server` giữ nguyên tên tool, quyền, token và giới hạn, so that các client hiện có không phải sửa.
51. As a maintainer, I want server và app dùng cùng một module ghi kho tri thức, so that sửa lỗi một lần là cả hai cùng được sửa.
52. As a maintainer, I want CI và thời gian build của server không bị dependency Tauri kéo chậm, so that vòng phát triển server vẫn nhanh.

## Implementation Decisions

### Vị trí và cấu trúc
- Mọi thứ nằm trong repo này (monorepo). App ở `apps/wiki-desktop`, hai crate mới ở `crates/`, package UI dùng chung ở `packages/`.
- Crate Rust của app Tauri **bị exclude khỏi Cargo workspace gốc** và có lockfile riêng. Nó tham chiếu tới crate dùng chung bằng `path`. Server không phải build Tauri.

### Module `wiki-core`: Vault (module sâu, seam chính)
- Bọc `llm_wiki::WikiEngine`. Caller không bao giờ nhìn thấy kiểu của `llm_wiki`.
- Interface: `open(config)`, các hàm đọc `tree(scope)`, `read(path, window)`, `search(query)`, `graph(focus?)`, `status()`, **một** hàm ghi `apply(change)` / `apply_from(origin, change)`, `subscribe()` trả về luồng sự kiện, và `notice_external(paths)` cho file watcher.
- `Change` là enum: `Write`, `Patch`, `Move`, `Delete`, `ImportRaw`, `IngestSource`, `Reindex`. Thêm khả năng mới thì thêm variant, không thêm hàm.
- `DocPath` là kiểu đã kiểm tra: đường dẫn tương đối, có Space, không `..`, không ra ngoài kho. Mọi lối vào (Tauri, MCP, server) đều đi qua kiểu này.
- Invariant:
  - `apply` thành công nghĩa là file đã trên đĩa, Search Projection và Graph Projection đã đồng bộ, `index.md`/`log.md` đã cập nhật và sự kiện đã phát.
  - Nếu thất bại thì không để lại trạng thái dở dang nhìn thấy được (ghi ra file tạm rồi rename).
  - Có một write lock trong process. Đọc không chờ lock.
- Optimistic concurrency theo Knowledge Revision (hash nội dung). `expect` không khớp thì trả lỗi `Conflict { current }`.
- **Một writer giữa các process:** advisory `flock` trên một lock file trong kho. Process không giữ lock được mở kho để đọc, còn `apply` trả `ReadOnly`.
- Lỗi: `NotFound`, `InvalidPath`, `Conflict`, `ReadOnly`, `Io`, `Index`.
- Sự kiện: `Changed { paths, origin: Ui|Mcp|Server }`, `External { paths }`, `Reindexed { stats }`.
- `Vault` giữ tập "path vừa ghi kèm hash" để watcher lọc các thay đổi do chính nó gây ra.
- Không tạo trait store để mock, vì chỉ có một adapter thật.

### Module `wiki-mcp`: ToolHost
- Tách từ phần dispatch tool knowledge MCP của `vela-server`. Interface: `new(vault, policy)`, `list_tools(access)`, `call(ctx, name, args)`.
- Giữ nguyên tên tool, phân loại đọc/ghi (theo `is_read_only_wiki_tool`), các variant lỗi hiện có, và thêm `ReadOnly`.
- `Policy`: `allow_write`, `max_concurrency` (mặc định 16), `call_timeout` (mặc định 30s), `max_result_chars` (giữ khoảng giới hạn hiện tại).
- Quản lý token và settings **không** thuộc ToolHost. `vela-server` giữ cách lưu hiện tại, app lưu token trong Keychain.
- Tool ghi map sang `Vault::apply_from(Origin::Mcp, …)`.
- `vela-server` chuyển `/mcp/knowledge` sang dùng ToolHost, hành vi bên ngoài giữ nguyên.

### Binary `wiki-mcp` (stdio)
- Đóng gói và ký cùng app.
- Chế độ **bridge**: đọc file runtime (port và token, quyền 0600, nằm trong Application Support của app) rồi chuyển tiếp JSON-RPC sang HTTP MCP của app bằng `rmcp` client.
- Chế độ **direct**: dùng khi không có app. Binary tự mở Vault, ghi được nếu lấy được lock, còn không thì chỉ đọc.

### Tauri shell
- Mỏng có chủ đích, không chứa logic nghiệp vụ.
- Sáu command tương ứng với Vault: `vault_tree`, `vault_read`, `vault_search`, `vault_graph`, `vault_status`, `vault_apply`. Thêm các command riêng của shell: mở hoặc chọn kho, `mcp_info`, `mcp_rotate_token`.
- Event: `vault://changed`, `vault://lock`, `mcp://status`.
- `Change` và `VaultError` serialize qua serde, nên frontend nhận lỗi có kiểu.
- File watcher dùng FSEvents (crate `notify`), debounce khoảng 300 ms, bỏ qua lock file, file SQLite và file tạm, rồi gọi `notice_external`.
- MCP HTTP dùng `rmcp` streamable HTTP server, chỉ bind 127.0.0.1, luôn yêu cầu bearer token và kiểm tra `Origin`/`Host`.
- Capabilities của WebView chỉ gồm dialog, event và các command trên. Không cấp `fs` hay `shell`. CSP không cho script ngoài.
- Settings của app (kho gần đây, port) lưu trong Application Support. Token lưu trong Keychain.
- Phát hành: universal binary, ký Developer ID với hardened runtime, notarize và đóng DMG.

### UI: seam KnowledgeBackend
- Interface TypeScript gồm `tree`, `read`, `search`, `graph`, `status`, `apply`, `onChange` và cờ `capabilities { chat }`. Cùng vốn từ với Vault.
- Có ba adapter: `HttpBackend` (Admin Web, gọi `/api/admin/knowledge/*` sẵn có), `TauriBackend` (invoke/listen) và `MemoryBackend` (test).
- Backend được inject qua provide/inject ở root. Component không import adapter cụ thể nào.
- Tách các component Knowledge, composable workspace, renderer Markdown/Mermaid/chart, CSS reader và các key i18n `knowledge.*` vào package dùng chung. Package này không phụ thuộc router, shell hay i18n của admin.
- Tab Wiki Chat bị ẩn khi `capabilities.chat = false`.

### Thuật ngữ
- Dùng thuật ngữ trong GLOSSARY: Wiki Page, Source, Raw Working Note, Knowledge Revision, Search Projection, Graph Projection, Wiki Link, Backlink, Knowledge Browser Space, Wiki Provenance.
- `Vault` là tên module, chỉ kho tri thức Wiki, **không phải Memory Vault**. Khi implement cần bổ sung mục này vào GLOSSARY để tránh nhầm.

### Thư viện
- Nếu cần sửa `llm-wiki-rs` (ví dụ để lộ revision), làm theo quy trình trong AGENTS.md: sửa ở repo lib, dùng `[patch.crates-io]` tạm, publish rồi gỡ patch.

## Testing Decisions

- Test tốt chỉ kiểm tra hành vi quan sát được qua interface của module, không kiểm tra chi tiết bên trong. Mỗi module có đúng một bề mặt test, cũng chính là interface của nó.
- **Seam test** (ba seam, mỗi seam là interface của một module, không thêm seam nào khác):
  1. **`Vault`**: phần lớn test nằm ở đây. Chạy trên kho thật trong thư mục tạm, không mock:
     - ghi → đọc/tìm thấy/nhận sự kiện;
     - sai revision → `Conflict`, file giữ nguyên;
     - hai `Vault` cùng mở một thư mục → cái thứ hai bị `ReadOnly`;
     - `DocPath` từ chối đường dẫn chứa `..` hoặc đường dẫn tuyệt đối;
     - `notice_external` sau khi sửa file trực tiếp → index được cập nhật và phát sự kiện `External`;
     - lần ghi của chính `Vault` không sinh ra sự kiện `External`.
  2. **`ToolHost`**:
     - quyền đọc/ghi, `allow_write = false`, timeout, cancel, giới hạn concurrency, cắt kết quả, lỗi `ReadOnly`;
     - một test tích hợp HTTP (`rmcp` client gọi server thật trên port ngẫu nhiên);
     - một test spawn binary stdio ở chế độ direct rồi gọi `tools/list` và `wiki_status`;
     - một test bridge: app giả lập một HTTP host, binary chuyển tiếp request.
  3. **`KnowledgeBackend`**: test Vitest cho component và composable Knowledge chạy trên `MemoryBackend` thay cho mock `fetch`. Một bộ contract test chung chạy cho cả `MemoryBackend` và `HttpBackend` (dùng server giả) để hai adapter hành xử giống nhau.
- Tauri shell không có unit test riêng vì quá mỏng. Kiểm tra bằng smoke test thủ công theo checklist: mở kho, sửa, sửa từ Obsidian, agent ghi qua MCP thì toast hiện trong vòng một giây.
- **Hồi quy:** toàn bộ test hiện có của knowledge MCP trong `vela-server` (wiki_status theo quyền, wiki_write_file bị chặn khi read-only, wiki_query, wiki_tree, wiki_get) phải pass trên crate mới và pass trong server. Vitest và E2E của trang Knowledge trong Admin Web phải pass sau khi tách package.
- **Prior art:** test knowledge MCP trong `vela-server` (gọi tool qua context có quyền), test trang Knowledge và `useWikiChat` trong Admin Web, test `KnowledgeGraph` và `RightPanelTabs`.
- Luôn chạy test qua `scripts/agent-test.sh`, chạy test của crate hoặc package đích trước, rồi mới chạy toàn workspace. Không gọi `cargo test` trực tiếp.

## Out of Scope

- Wiki Chat và Wiki Conversation trong app (cần LLM provider và lưu hội thoại). Để v2.
- Embeddings, reranking, hybrid search vector.
- AI ingest và Wiki Auto-Flow dùng LLM, cùng hàng đợi review.
- Đồng bộ nhiều máy do app tự làm. Người dùng tự dùng Git/iCloud trên thư mục kho.
- Memory Vault và mọi thao tác Memory.
- Cập nhật tự động (updater) có thể làm sau bản phát hành đầu tiên.
- Phiên bản Windows/Linux. Tauri hỗ trợ, nhưng chưa nghiệm thu.
- Chuyển các route `/api/admin/knowledge/*` của server sang dùng `Vault`. Việc này nên làm, nhưng tách thành issue riêng sau khi app ổn định. Spec này chỉ bắt buộc chuyển `/mcp/knowledge`.

## Further Notes

- Thứ tự triển khai (mỗi bước merge độc lập): `wiki-core` → `wiki-mcp` + chuyển server sang dùng + binary stdio direct → seam `KnowledgeBackend` + tách package (Admin Web giữ nguyên giao diện) → Tauri shell MVP → MCP HTTP trong app + bridge + Settings MCP → ký/notarize/DMG. Chi tiết và tiêu chí xong của từng bước ở `docs/wiki-desktop/implementation-plan.md`.
- Rủi ro lớn nhất là nhiều process cùng ghi một Search Projection SQLite. Mô hình một writer bằng `flock` cộng với bridge là điều kiện bắt buộc, không phải tối ưu hóa.
- Spec này được chốt theo các đề xuất trong phiên thiết kế ngày 2026-11-05 và 2026-11-08.

## Tickets

| # | Ticket | Blocked by |
|---|---|---|
| 01 | [Vault: mở kho và đọc](issues/01-vault-open-and-read.md) | — |
| 02 | [Vault: ghi trọn vẹn, revision, sự kiện](issues/02-vault-apply-revisions-events.md) | 01 |
| 03 | [Vault: một writer, thay đổi bên ngoài](issues/03-vault-single-writer-and-external-changes.md) | 02 |
| 04 | [ToolHost + chuyển vela-server](issues/04-toolhost-extract-and-server-switch.md) | 02 |
| 05 | [wiki-mcp stdio direct](issues/05-wiki-mcp-stdio-direct.md) | 03, 04 |
| 06 | [Seam KnowledgeBackend trong Admin Web](issues/06-knowledgebackend-seam-in-admin-web.md) | — |
| 07 | [Tách package knowledge-ui](issues/07-extract-knowledge-ui-package.md) | 06 |
| 08 | [Tauri: mở kho, duyệt chỉ đọc](issues/08-tauri-shell-open-and-browse.md) | 01, 07 |
| 09 | [Tauri: soạn, xung đột, sự kiện](issues/09-tauri-shell-editing-and-events.md) | 02, 08 |
| 10 | [Tauri: file watcher](issues/10-tauri-file-watcher.md) | 03, 09 |
| 11 | [MCP HTTP trong app + Settings MCP](issues/11-in-app-mcp-http.md) | 04, 09 |
| 12 | [wiki-mcp bridge](issues/12-wiki-mcp-bridge-mode.md) | 05, 11 |
| 13 | [Ký, notarize, DMG](issues/13-sign-notarize-dmg.md) (ready-for-human) | 12 |
