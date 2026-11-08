# wiki-mcp: module ToolHost

Crate mới `crates/wiki-mcp`, tách ra từ `crates/vela-server/src/tools/knowledge_mcp/` (`mod.rs`, `settings.rs`, `staging.rs`, khoảng 1.300 dòng không tính test).

## Interface

```rust
pub struct ToolHost { /* Vault, Policy, limits */ }

impl ToolHost {
    pub fn new(vault: Arc<Vault>, policy: Policy) -> ToolHost;

    /// Các tool mà caller có quyền thấy (read-only hoặc read-write).
    pub fn list_tools(&self, access: Access) -> Vec<ToolSpec>;

    /// Chạy một tool. Tự xử lý quyền, timeout, cancel, giới hạn concurrency
    /// và cắt bớt kết quả quá dài.
    pub async fn call(&self, ctx: CallCtx, name: &str, args: serde_json::Value)
        -> Result<ToolOutput, ToolError>;
}

pub enum Access { ReadOnly, ReadWrite }
pub struct CallCtx { pub access: Access, pub cancel: CancellationToken, pub deadline: Instant }
```

Interface này giống hệt `KnowledgeMcp::list_tools` và `KnowledgeMcp::call_tool` hiện tại, nên việc tách crate chủ yếu là di chuyển code. Phần quản lý token và settings (`create_token`, `delete_token`, `update_settings`, `admin_snapshot`) **không** thuộc `ToolHost`. Đó là việc của từng nơi host: server lưu token theo cách của server, app lưu trong Keychain.

## Danh sách tool

Giữ nguyên tên tool đang có để agent và prompt hiện tại (`docs/knowledge/client-agent-wiki-kit/`) dùng được ngay:

| Read-only | Read-write |
|---|---|
| `wiki_tree`, `wiki_query`, `wiki_get`, `wiki_multi_get`, `wiki_metadata`, `wiki_status`, `wiki_graph_neighbors` | `wiki_write_file`, `wiki_patch_file`, `wiki_file_synthesis`, `wiki_ingest_source` |

Phân loại dựa vào `llm_wiki::is_read_only_wiki_tool`. Tool đọc gọi `Vault::read`/`search`/`tree`/`graph`. Tool ghi map sang `Vault::apply(Change::…)` với `Origin::Mcp`.

## Hai transport, một ToolHost

```
Claude Desktop ─stdio─► wiki-mcp (bin) ─┬─ app đang chạy? ─► HTTP MCP của app ─► ToolHost
                                       └─ không ──────────► Vault (read-only nếu bị lock) ─► ToolHost
Cursor / Vela ─streamable HTTP─► app 127.0.0.1:<port>/mcp ─► ToolHost
```

- **HTTP (trong app):** dùng `rmcp` 3.5 với `transport-streamable-http-server` (workspace đã có sẵn feature). Chỉ bind `127.0.0.1`. Header `Authorization: Bearer <token>` quyết định `Access`. App tự sinh token và lưu trong Keychain.
- **stdio (`wiki-mcp` binary):** đi kèm trong bundle tại `Wiki.app/Contents/MacOS/wiki-mcp`. Có hai chế độ:
  - **bridge** (mặc định khi app đang chạy): đọc `~/Library/Application Support/<bundle-id>/runtime.json` (port và token, quyền file `0600`), rồi chuyển tiếp JSON-RPC sang HTTP. Dùng `rmcp` client.
  - **direct** (khi app không chạy): tự `Vault::open`. Nếu lấy được lock thì có quyền ghi, nếu không thì chỉ đọc.

## Một writer <a id="mot-writer"></a>

Index SQLite và `index.md`/`log.md` không chịu được hai process cùng ghi. Quy tắc:

1. Process nào giữ `flock` trên `.wiki-desktop.lock` thì process đó là writer.
2. App luôn cố lấy lock khi mở vault. Nếu `wiki-mcp` direct đang giữ lock thì app mở ở chế độ chỉ đọc, hiện banner, và thử lấy lại lock mỗi vài giây.
3. Khi app đang chạy, bridge giúp mọi request MCP đi qua app, nên UI nhận được `VaultEvent` ngay lập tức.

Bị giới hạn, ít trạng thái, không cần daemon riêng.

## Policy

```rust
pub struct Policy {
    pub allow_write: bool,        // tắt thì client ReadWrite cũng chỉ thấy tool đọc
    pub max_concurrency: usize,   // mặc định 16, giống WikiMcpConfig
    pub call_timeout: Duration,   // mặc định 30s
    pub max_result_chars: usize,  // giữ RESULT_CHARS_RANGE hiện tại
}
```

## Lỗi

`ToolError` giữ nguyên các variant của `KnowledgeToolError` (`Denied`, `InvalidArguments`, `ExecutionFailed`, `Unavailable`, `Cancelled`, `Timeout`) và có thêm `ReadOnly`. Lỗi này trả về khi vault đang bị process khác giữ lock ghi.

## Test

- Chuyển `knowledge_mcp/tests.rs` sang `wiki-mcp`, đổi `WikiEngine` thành `Vault` trên `tempdir`.
- Test transport: một test HTTP (rmcp client gọi server thật trên port ngẫu nhiên) và một test stdio direct (spawn binary và gửi `tools/list`).
- `vela-server` vẫn phải pass bộ test hiện có sau khi chuyển sang dùng `wiki-mcp`.

## Cấu hình client (tài liệu cho người dùng)

```json
{ "mcpServers": { "wiki": { "command": "/Applications/Wiki.app/Contents/MacOS/wiki-mcp" } } }
```

App có màn Settings → MCP để hiện đoạn cấu hình này, nút copy URL HTTP kèm token, và nút "Cài vào Claude Desktop". Nút này ghi vào `claude_desktop_config.json` và phải hỏi người dùng xác nhận trước.
