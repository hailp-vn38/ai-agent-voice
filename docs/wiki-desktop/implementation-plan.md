# Kế hoạch triển khai

Mỗi giai đoạn tạo một nhánh riêng và merge độc lập. Test luôn chạy qua `scripts/agent-test.sh`.

## Giai đoạn 1: `wiki-core`

- Tạo `crates/wiki-core` với `Vault`, `DocPath`, `Change`, `VaultError`, `VaultEvent` và advisory lock.
- Viết test như mô tả trong [core.md](core.md#cach-test).
- **Xong khi:** test pass. `vela-server` chưa phải sửa gì.

## Giai đoạn 2: `wiki-mcp`

- Chuyển `knowledge_mcp/{mod,staging}.rs` sang `crates/wiki-mcp::ToolHost` chạy trên `Vault`. `settings.rs` (token, settings) vẫn ở `vela-server`.
- Cho `vela-server` dùng `wiki-mcp`. Route `/mcp/knowledge` giữ nguyên hành vi.
- Thêm binary `wiki-mcp` chế độ direct (stdio).
- **Xong khi:** bộ test `knowledge_mcp` hiện có pass trên crate mới, test của `vela-server` pass, và Claude Code gọi được `wiki_status` qua stdio.

## Giai đoạn 3: UI dùng chung

- Định nghĩa `KnowledgeBackend` và `HttpBackend`. Refactor `admin-web` để đi qua seam này. Không đổi giao diện.
- Tách thành `packages/knowledge-ui` cùng `MemoryBackend` cho test.
- **Xong khi:** Vitest và E2E của `admin-web` pass, trang Knowledge trên server không thay đổi gì.

## Giai đoạn 4: Tauri shell (MVP)

- Dựng `apps/wiki-desktop` với sáu command, event, watcher, chọn vault và `TauriBackend`.
- **Xong khi:** mở được vault, duyệt, đọc, sửa, tìm, xem graph. Sửa file trong Obsidian thì UI tự cập nhật.

## Giai đoạn 5: MCP trong app

- HTTP MCP trên loopback, token trong Keychain, `runtime.json`, chế độ bridge của `wiki-mcp`, màn Settings → MCP.
- **Xong khi:** agent ghi trang qua MCP thì UI hiện toast trong vòng 1 giây. Mở đồng thời app và `wiki-mcp` direct thì không làm hỏng index (có test tích hợp).

## Giai đoạn 6: Phát hành

- Build universal, ký, notarize, tạo DMG. Có thể thêm updater.

## v2 <a id="v2"></a>

- Wiki Chat: tận dụng `build_query_agent_system_prompt` của `llm-wiki-rs`, provider cấu hình trong app (API key lưu Keychain).
- Embeddings và hybrid search (`with_embedder`, `embed_pending`).
- AI ingest (`build_ingest_*` prompts) với hàng đợi review.

## Rủi ro

| Rủi ro | Giảm thiểu |
|---|---|
| `llm-wiki-rs` không chịu được nhiều process ghi | Mô hình một writer bằng `flock` và bridge ([mcp.md](mcp.md#mot-writer)) |
| Refactor `admin-web` làm hỏng trang Knowledge hiện tại | Giai đoạn 3 không đổi UI, chạy đủ test cũ trước khi tách package |
| Cần sửa `llm-wiki-rs` (ví dụ thêm hook revision) | Làm theo quy trình trong AGENTS.md: sửa ở `~/Desktop/project_rust/libs/`, `[patch.crates-io]` tạm, publish rồi gỡ patch |
| Trang web độc hại gọi MCP localhost | Bắt buộc token, kiểm tra `Origin`/`Host` ([desktop-shell.md](desktop-shell.md#bao-mat)) |
| Watcher phát sự kiện lặp với ghi nội bộ | Lọc theo tập "path vừa ghi kèm hash" |
