# wiki-core: module Vault

Crate mới `crates/wiki-core`. Đây là **module sâu** duy nhất mà cả Tauri shell, `wiki-mcp` và (về sau) `vela-server` gọi khi cần làm gì đó với vault.

## Vì sao cần module này

Hiện tại `vela-server` gọi thẳng `llm_wiki::WikiEngine`. `WikiEngine` có hơn 40 hàm public (`write_source_file`, `write_wiki_file`, `write_file`, `delete_file`, `delete_wiki_file`, `delete_source_file`, `sync_index`, `rebuild_or_sync_index_md`, `append_log`, …). Để dùng đúng, caller phải tự biết thứ tự gọi: ghi file → `sync_index` → cập nhật `index.md` → `append_log` → báo UI.

**Deletion test:** nếu không có `wiki-core`, quy trình đó phải lặp lại ở Tauri commands, ở MCP tools và ở các route admin. Có module này thì quy trình nằm ở một chỗ (**locality**), và mọi caller chỉ cần học một interface nhỏ (**leverage**).

## Interface

```rust
pub struct Vault { /* Arc<WikiEngine>, write lock, change bus */ }

impl Vault {
    /// Mở hoặc tạo vault. Tự đồng bộ index nếu index cũ hơn file trên đĩa.
    pub fn open(cfg: VaultConfig) -> Result<Vault, VaultError>;

    // --- Đọc (chạy song song được, không giữ write lock) ---
    pub fn tree(&self, scope: Scope) -> Result<TreeNode, VaultError>;
    pub fn read(&self, path: &DocPath, window: ReadWindow) -> Result<Page, VaultError>;
    pub fn search(&self, q: SearchQuery) -> Result<Vec<Hit>, VaultError>;
    pub fn graph(&self, focus: Option<&DocPath>) -> Result<Graph, VaultError>;
    pub fn status(&self) -> Result<VaultStatus, VaultError>;

    // --- Ghi (tuần tự hóa nội bộ, mỗi lần gọi là một thao tác trọn vẹn) ---
    pub fn apply(&self, change: Change) -> Result<Applied, VaultError>;

    // --- Theo dõi thay đổi ---
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<VaultEvent>;
}

pub enum Scope { Raw, Wiki, All }

pub enum Change {
    Write  { path: DocPath, content: String, expect: Option<Revision> },
    Patch  { path: DocPath, edits: Vec<TextEdit>, expect: Option<Revision> },
    Move   { from: DocPath, to: DocPath },
    Delete { path: DocPath },
    ImportRaw { files: Vec<RawFile> },            // giống /raw/batch
    IngestSource { source: DocPath },             // wiki_ingest_source, không dùng LLM
    Reindex { force: bool },                      // giống /index/rebuild
}
```

Chỉ có **một** hàm ghi là `apply(Change)`. Thêm một loại thay đổi mới thì thêm một variant, interface không phình ra. Tauri commands và MCP tools đều map vào đây.

`DocPath` là một kiểu đã được kiểm tra: đường dẫn tương đối, có `Scope`, không chứa `..`, không trỏ ra ngoài vault. Tạo được `DocPath` nghĩa là đường dẫn đã an toàn, nên caller không phải tự sanitize nữa.

## Invariant (phần ngầm của interface mà caller cần biết)

1. **Thao tác trọn vẹn.** Khi `apply` trả `Ok`, file đã nằm trên đĩa, index SQLite đã đồng bộ, `index.md` và `log.md` đã cập nhật, và một `VaultEvent` đã được phát. Khi trả `Err`, không có bước nào để lại trạng thái dở dang mà caller nhìn thấy được (ghi ra file tạm rồi rename).
2. **Một writer trong process.** Mọi lệnh `apply` đi qua một write lock nội bộ. Đọc không cần chờ lock.
3. **Một writer giữa các process.** `open` lấy advisory lock (`flock`) trên `<vault>/.wiki-desktop.lock`. Process thứ hai mở được vault ở chế độ chỉ đọc, còn `apply` trả `VaultError::ReadOnly`. Xem [mcp.md](mcp.md#mot-writer).
4. **Kiểm soát xung đột ghi.** Nếu `expect: Some(rev)` không khớp revision hiện tại thì trả `Conflict { current }`. UI dùng cơ chế này khi người dùng đang sửa trang mà agent cũng ghi vào trang đó.
5. **Thay đổi từ bên ngoài.** File bị sửa ngoài app (Obsidian, Git, Finder) không đi qua `apply`. Tauri shell báo bằng `Vault::notice_external(paths)` (internal seam, xem [desktop-shell.md](desktop-shell.md#file-watcher)). Hàm này sync index và phát `VaultEvent::External`.
6. **Chi phí.** `read` và `search` là truy vấn SQLite cục bộ, thường dưới 10 ms. `graph(None)` có thể chậm với vault lớn nên UI phải gọi bất đồng bộ.

## Lỗi

```rust
pub enum VaultError {
    NotFound(DocPath),
    InvalidPath(String),
    Conflict { current: Revision },
    ReadOnly,                 // process khác đang giữ lock ghi
    Io(String),
    Index(String),            // lỗi từ WikiEngine/SQLite
}
```

Có `impl From<WikiError> for VaultError`, nên caller không bao giờ thấy kiểu của `llm_wiki`. Nhờ vậy có thể nâng `llm-wiki-rs` lên phiên bản mới mà caller không phải sửa gì.

## Sự kiện

```rust
pub enum VaultEvent {
    Changed  { paths: Vec<DocPath>, origin: Origin }, // Origin::{Ui, Mcp, Server}
    External { paths: Vec<DocPath> },
    Reindexed { stats: SyncStats },
}
```

Caller truyền `Origin` qua `Change` context (`apply_from(origin, change)`, một biến thể của `apply`). UI dựa vào nó để hiện thông báo kiểu "Agent vừa sửa trang này".

## Seam bên trong (không thuộc interface)

- `Store`: phần bọc `WikiEngine`. Chỉ có **một adapter thật** nên không tách thành trait. Test chạy trên vault thật trong `tempdir`, vì `WikiEngine::open` nhanh và chỉ cần SQLite cục bộ.
- `Clock`/`Revision`: revision là hash nội dung, không dùng thời gian, nên không cần fake clock.

> Theo nguyên tắc "một adapter là seam giả định": không tạo `trait WikiStore` chỉ để mock. Interface `Vault` chính là bề mặt test.

## Cách test <a id="cach-test"></a>

Tất cả test đi qua `Vault`, chạy bằng `scripts/agent-test.sh` (không gọi `cargo test` trực tiếp):

- `apply(Write)` → `read` trả đúng nội dung, `search` tìm thấy, `subscribe` nhận được `Changed`.
- `expect` sai revision → `Conflict`, file giữ nguyên.
- Mở hai `Vault` trên cùng thư mục → cái thứ hai bị `ReadOnly` khi ghi.
- `DocPath::parse("../x")` → `InvalidPath`.
- Bộ test hồi quy lấy từ `knowledge_mcp/tests.rs` hiện tại (`wiki_tree`, `wiki_query`, `wiki_get`) chuyển sang chạy trên `Vault`.

## Quan hệ với vela-server

Giai đoạn 1 chỉ tạo crate, chưa sửa `vela-server`. Giai đoạn 3 (tùy chọn) cho các route `/api/admin/knowledge/*` gọi `Vault` thay vì gọi `WikiEngine` trực tiếp. Lúc đó server và app có cùng hành vi ghi.
