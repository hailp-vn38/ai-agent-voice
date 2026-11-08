# Wiki Desktop (macOS, Tauri 2)

Status: **proposed** (2026-11-05). Chưa có code.

App macOS độc lập để đọc, viết và tìm kiếm Wiki với UI giống trang Knowledge của Admin Web. App có MCP server riêng để các agent (Claude Desktop/Code, Cursor, Vela) đọc và ghi cùng vault.

## Quyết định chính

- **Vỏ app:** Tauri 2. WebView chạy Vue 3 (lấy lại từ `apps/admin-web/src/pages/knowledge`), backend là Rust.
- **Lõi wiki:** `llm-wiki-rs` (`WikiEngine`) chạy trong process của app. Không cần `vela-server` hay Docker.
- **MCP:** dùng lại phần định nghĩa và dispatch tool đang có trong `crates/vela-server/src/tools/knowledge_mcp/`, tách ra crate riêng để server và app dùng chung.
- **Một process sở hữu vault.** Khi app đang mở, chỉ app ghi vault. Binary stdio `wiki-mcp` chuyển tiếp request sang app. Xem [mcp.md](mcp.md#mot-writer).

## Sơ đồ module

```
apps/wiki-desktop (Tauri shell)
 ├─ UI Vue  ──KnowledgeBackend seam──►  TauriBackend ──invoke()──┐
 │                                                               ▼
 ├─ src-tauri ─────────────────────────────────────────►  wiki-core::Vault
 │     └─ MCP HTTP 127.0.0.1 (rmcp) ──► wiki-mcp::ToolHost ─────┘
 └─ bin wiki-mcp (stdio) ──bridge──► MCP HTTP của app | fallback: Vault trực tiếp

crates/vela-server ──► wiki-mcp::ToolHost (thay knowledge_mcp hiện tại)
```

## Tài liệu

| File | Nội dung |
|---|---|
| [core.md](core.md) | Module `wiki-core`: interface `Vault`, các invariant, lỗi, sự kiện thay đổi, cách test |
| [mcp.md](mcp.md) | Module `wiki-mcp`: `ToolHost`, chính sách truy cập, hai transport, mô hình một writer |
| [desktop-shell.md](desktop-shell.md) | Tauri shell: commands, events, file watcher, cấu hình, đóng gói và ký app |
| [ui-reuse.md](ui-reuse.md) | Dùng lại UI Vue: seam `KnowledgeBackend` với hai adapter (HTTP và Tauri) |
| [implementation-plan.md](implementation-plan.md) | Các giai đoạn, tiêu chí nghiệm thu, rủi ro |

## Thuật ngữ

Thiết kế dùng từ vựng của skill `codebase-design`: **module**, **interface**, **seam**, **adapter**, **depth**, **leverage**, **locality**. "Vault" là thư mục dữ liệu wiki mà `WikiEngine::open(data_root)` mở, gồm các nguồn Raw, các trang Wiki và index SQLite dẫn xuất.

## Ngoài phạm vi v1

- Wiki Chat (`/api/admin/knowledge/ask`, `conversations`): cần LLM provider và lưu hội thoại. Để v2, xem [implementation-plan.md](implementation-plan.md#v2).
- AI ingest tự động và embeddings/reranking. `WikiEngine::with_embedder` để dành cho sau.
- Đồng bộ nhiều máy. Vault là thư mục thường nên người dùng có thể tự đồng bộ bằng iCloud hoặc Git.
