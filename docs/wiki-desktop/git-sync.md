# Đồng bộ vault lên GitHub

Status: **proposed** (2026-11-09). Chưa có code. Ticket: `.scratch/wiki-git-sync/`.

Vault Markdown được lưu trữ lên một repo GitHub (nên để private) và kéo thay đổi từ nơi khác về. Có hai host dùng chung một module:

- **App Wiki Desktop**: sync vault trên máy người dùng, cấu hình trong Settings của app.
- **`vela-server` + Admin Web** (`apps/admin-web`): sync vault của server (`[wiki] vault_dir`, trong Docker là `/app/data/wiki-vault`). Cấu hình nằm trong `config.toml`; Admin Web xem trạng thái, bấm "Sync now" và xem xung đột. Xem [Trên vela-server và Admin Web](#server).

Người dùng không phải tự gõ lệnh git ở cả hai nơi.

## Quyết định chính

- **Dùng `git` CLI của hệ thống**, không nhúng `git2`/`gix` và không gọi GitHub REST API. Xem [So sánh](#so-sanh).
- **Module sâu mới `wiki-core::sync::VaultSync`.** Tauri shell chỉ nối command, event và lịch chạy. Toàn bộ quy trình commit → fetch → merge → push nằm trong module này (**locality**).
- **Sync không phải một `Change`.** Nó không thao tác lên một tài liệu cụ thể. `Vault` chỉ thêm một seam nội bộ để `VaultSync` giữ write lock trong lúc merge.
- **Chỉ lock holder được sync.** Process đang giữ `flock` trên `.wiki-desktop.lock` mới được chạy git. Reader (app khi `wiki-mcp` direct giữ lock) tắt sync và hiện lý do.
- **Xung đột thì giữ cả hai bản.** Sync không bao giờ dừng ở trạng thái merge dở và không làm mất nội dung.
- **Xác thực theo host.** Desktop dùng credential git sẵn có (SSH key, `osxkeychain`, `gh auth setup-git`). Server dùng một GitHub deploy key (SSH, có quyền ghi, chỉ cho đúng một repo) mount read-only vào container. PAT để v2.
- **UI sync nằm trong package dùng chung `packages/knowledge-ui`**, sau một seam `SyncBackend` có hai adapter (Tauri và HTTP admin), giống seam `KnowledgeBackend` đang có.

## So sánh <a id="so-sanh"></a>

| Cách | Ưu | Nhược |
|---|---|---|
| **A. `git` CLI (chọn)** | Dùng luôn cấu hình xác thực của người dùng. Merge, merge driver (`union`), LFS và hook đầy đủ. Ít code. | Máy phải có git (Xcode Command Line Tools). Phải đọc output của git. |
| B. `git2` / `gix` nhúng | Không phụ thuộc git hệ thống | Phải tự làm xác thực. `git2` kéo libgit2 + OpenSSL + libssh2, khó build universal và ký. Push/merge của `gix` chưa đủ. |
| C. GitHub REST (octocrab) | Không cần git | Mỗi file một request, dính rate limit, không có merge, không hợp file nhị phân. |

## Interface

```rust
pub struct VaultSync { /* Arc<Vault>, SyncConfig, trạng thái, broadcast */ }

impl VaultSync {
    /// Kiểm tra git, init repo nếu chưa có, ghi .gitignore/.gitattributes, đặt remote.
    pub fn configure(vault: Arc<Vault>, cfg: SyncConfig) -> Result<VaultSync, SyncError>;

    /// Một lần đồng bộ trọn vẹn. Chặn (blocking); shell gọi qua spawn_blocking.
    pub fn sync_now(&self) -> Result<SyncReport, SyncError>;

    pub fn status(&self) -> SyncStatus;
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<SyncStatus>;
}

pub struct SyncConfig {
    pub remote_url: Option<String>, // None: dùng remote `origin` đã có trong .git/config
    pub branch: String,             // mặc định "main"
    pub author: Option<GitAuthor>,  // None: dùng user.name/user.email của git
    pub auth: GitAuth,
    pub max_file_bytes: u64,        // mặc định 50 MB
    pub command_timeout: Duration,  // mặc định 60 s cho mỗi lệnh git
}

pub enum GitAuth {
    /// Desktop: dùng cấu hình git và credential helper của người dùng.
    System,
    /// Server: chạy headless. Đặt GIT_SSH_COMMAND với `-i <key> -o IdentitiesOnly=yes
    /// -o UserKnownHostsFile=<known_hosts> -o StrictHostKeyChecking=yes`,
    /// GIT_CONFIG_GLOBAL=/dev/null, `-c safe.directory=<vault>`.
    SshKey { key: PathBuf, known_hosts: PathBuf },
}

pub enum SyncStatus {
    Disabled { reason: DisabledReason }, // NoGit | NotConfigured | ReadOnly
    Idle { last_sync: Option<SystemTime>, ahead: u32, behind: u32 },
    Syncing,
    Deferred { reason: DeferReason, retry_at: SystemTime }, // server: IngestRunning
    Conflicted { copies: Vec<DocPath> }, // đã tạo bản conflict, chờ người dùng gộp
    Error { kind: SyncErrorKind, message: String },
}

pub struct SyncReport {
    pub committed: usize,      // số file trong commit local
    pub pulled: Vec<DocPath>,  // file đổi do merge từ remote
    pub conflicts: Vec<DocPath>,
    pub pushed: bool,
}

pub enum SyncError {
    GitMissing,
    ReadOnly,
    Auth(String),          // git trả lỗi xác thực; không bao giờ chứa token
    Network(String),
    Timeout { command: String },
    FileTooLarge { path: DocPath, bytes: u64 },
    UnsafeRemote(String),  // URL chứa credential
    Git(String),
    Vault(VaultError),
}
```

`SyncStatus`, `SyncReport` và `SyncError` derive `serde` để frontend nhận lỗi có kiểu, giống `VaultError`.

### Seam mới trên `Vault`

```rust
impl Vault {
    /// Giữ write lock, chạy `f` trên thư mục vault, rồi coi các path `f` trả về
    /// là thay đổi bên ngoài: ghi revision vào self_writes (watcher sẽ lọc),
    /// sync index, phát VaultEvent::External. Trả ReadOnly nếu không giữ flock.
    pub(crate) fn exclusive_external<F>(&self, f: F) -> Result<Vec<DocPath>, VaultError>
    where F: FnOnce(&Path) -> Result<Vec<PathBuf>, VaultError>;
}
```

Hàm này là `pub(crate)` vì `VaultSync` nằm cùng crate, nên interface công khai của `Vault` không đổi.

## Một lần `sync_now`

1. Nếu vault không writable thì trả `ReadOnly`. Nếu đang có một lần sync khác thì chờ lần đó xong, không chạy song song.
2. Vào `exclusive_external`. Trong closure:
   1. Quét các file sắp được thêm vào commit. Nếu có file lớn hơn `max_file_bytes` thì dừng với `FileTooLarge`, không commit gì.
   2. `git add -A` rồi `git commit` nếu có thay đổi. Message: `vault: <n> files from <hostname>`. Nếu `VaultEvent` từ lần sync trước tới giờ có origin `mcp`, thêm dòng `Origin: ui, mcp`.
   3. Ghi lại `old = HEAD`, chạy `git fetch origin <branch>`.
   4. `git merge --no-edit origin/<branch>`. Không dùng rebase.
   5. Nếu merge xung đột thì xử lý theo [Xung đột](#xung-dot), rồi commit merge.
   6. Trả về `git diff --name-only <old> HEAD`, lọc chỉ giữ path dưới `sources/` và `wiki/`.
3. `exclusive_external` sync index và phát `External` cho các path đó, nên UI và kết quả search cập nhật.
4. `git push origin <branch>`, ngoài write lock vì push không đổi file trên đĩa. Nếu push bị từ chối (remote vừa có commit mới) thì quay lại bước 2 một lần, sau đó báo lỗi.
5. Cập nhật `SyncStatus::Idle` với `ahead`/`behind` từ `git rev-list --left-right --count`.

Mọi lệnh git chạy với `GIT_TERMINAL_PROMPT=0`, `GIT_ASKPASS=` rỗng (v1), `LC_ALL=C` để đọc output ổn định, và bị kill khi quá `command_timeout`.

## File được đưa lên git

`configure` tạo hoặc bổ sung (không ghi đè dòng người dùng đã có):

`.gitignore`
```
.wiki/
.wiki-desktop.lock
*.tmp
*.temp
.obsidian/workspace*.json
.DS_Store
```

`.gitattributes`
```
wiki/log.md merge=union
```

- Đưa lên: `SCHEMA.md`, `sources/`, `wiki/`, phần còn lại của `.obsidian/`.
- Không đưa lên: Search Projection (`.wiki/index.sqlite*`), vì nó dẫn xuất được và mỗi máy tự dựng lại.
- `wiki/log.md` chỉ ghi nối thêm, nên merge `union` giữ dòng của cả hai bên là đúng.
- `wiki/index.md` cũng dẫn xuất. Nếu nó xung đột thì lấy bản local, rồi sau merge chạy `Change::Reindex { force: false }` để sinh lại.

## Xung đột <a id="xung-dot"></a>

Với mỗi file xung đột (trừ `wiki/index.md` và `wiki/log.md` đã nói ở trên):

1. Bản local giữ nguyên tên và nội dung (`git checkout --ours`).
2. Bản remote ghi thành `<tên> (conflict <YYYY-MM-DD> <host-remote>).md` cạnh file gốc. Host lấy từ commit remote; không có thì dùng `remote`.
3. Xóa/sửa xung đột (một bên xóa, một bên sửa): giữ bản đã sửa.
4. Commit merge, đưa các bản conflict vào `SyncStatus::Conflicted { copies }`.

Người dùng mở hai file, gộp bằng tay rồi xóa bản conflict. Lần sync kế tiếp thấy không còn file conflict nào thì trở về `Idle`. Cách này giống Obsidian Git và Dropbox: không mất dữ liệu và sync không bị kẹt.

## Xác thực và bảo mật

- v1 chỉ dùng credential git đã có. App chạy `git ls-remote` khi cấu hình để báo sớm lỗi `Auth` kèm hướng dẫn (`gh auth setup-git` hoặc thêm SSH key).
- **Từ chối URL chứa credential** (`https://user:token@…`) với `UnsafeRemote`, vì token sẽ nằm lại trong `.git/config` và có thể bị đồng bộ đi nơi khác.
- Thông báo lỗi lấy từ stderr của git phải được lọc bỏ mọi chuỗi giống credential trước khi tới UI hoặc log.
- Nếu có `gh`, kiểm tra `gh repo view --json visibility`; repo public thì hiện cảnh báo, nhưng không chặn.
- v2: PAT trong Keychain (`keyring`, đã có trong app), đưa cho git qua một helper `GIT_ASKPASS` đi kèm app.

## Tauri shell

| Command | Gọi tới |
|---|---|
| `sync_status()` | `VaultSync::status` |
| `sync_now()` | `VaultSync::sync_now` (spawn_blocking) |
| `sync_configure(remote_url, branch, auto, interval_min)` | `VaultSync::configure` + lưu settings |
| `sync_disable()` | dừng lịch, giữ nguyên `.git` |

Event `sync://status`: `SyncStatus` đã serialize, mỗi khi trạng thái đổi.

**Lịch chạy** (trong shell, mỗi vault một task, hủy khi đổi vault):
- Khi mở vault: sync một lần.
- Sau `VaultEvent::Changed` (origin Ui hoặc Mcp): chờ 30 s không có thay đổi mới rồi sync.
- Định kỳ: mặc định 5 phút để kéo thay đổi từ máy khác.
- Khi app thoát: nếu có thay đổi chưa commit thì sync một lần, tối đa 10 s.
- Tắt `auto` thì chỉ còn nút "Sync now".

**Settings** (`settings.json`): thêm `sync: { "<đường dẫn vault>": { auto, interval_min } }`. Remote và branch không lưu ở đây mà đọc từ `.git/config` của vault, để mở vault bằng git ngoài app vẫn đúng.

**Watcher:** không cần sửa. `.git/` nằm ngoài `sources/` và `wiki/` nên đã bị bỏ qua ([watcher.rs](../../apps/wiki-desktop/src-tauri/src/watcher.rs)). File do merge ghi được `exclusive_external` đánh dấu trong `self_writes`, nên watcher không báo lặp.

## Trên vela-server và Admin Web <a id="server"></a>

### Khác biệt so với desktop

1. **Server có nhiều writer không đi qua `Vault`.** Các route `/api/admin/knowledge/*` và job AI ingest (`orchestrator-flow`, `task_flow/sources_watch`) gọi thẳng `Arc<WikiEngine>`. Chỉ tool MCP đi qua `wiki_core::Vault`. Vì thế write lock của `Vault` không đủ chặn ghi trong lúc merge.
2. **`Vault` chưa phải một thể hiện dùng chung.** `KnowledgeMcp` tự `Vault::open` trong constructor ([knowledge_mcp/mod.rs](../../crates/vela-server/src/tools/knowledge_mcp/mod.rs)).
3. **Image runtime chưa có `git` và `openssh-client`** ([server.Dockerfile](../../docker/server.Dockerfile)). Container chạy user `1000`, có thể không có `HOME`, và chủ sở hữu thư mục vault có thể khác uid, nên git sẽ báo `dubious ownership`.
4. **Admin Web đã tự làm mới** khi file đổi nhờ SSE `GET /knowledge/events`. Sau khi pull, UI tự tải lại mà không cần thêm gì.

### Thiết kế

- **Một `Arc<Vault>` trong `AppState`**, mở một lần khi `[wiki] enabled`. `KnowledgeMcp` và `VaultSync` dùng chung thể hiện này.
- **`KnowledgeWriteGate`**: một `tokio::sync::RwLock<()>` trong `AppState`.
  - Một middleware axum bọc các route `/knowledge/*` có method `POST`/`PUT`/`PATCH`/`DELETE`, và tool MCP ghi; mỗi request giữ read guard (nhiều writer chạy song song như hiện tại).
  - `VaultSync::sync_now` chạy trong write guard, nên trong lúc merge không request ghi nào chạy. Request đến trong lúc đó chờ, tối đa bằng `command_timeout`.
  - Các route đọc và SSE không bị chặn.
- **Job AI ingest**: không chặn được từng lần ghi bên trong `orchestrator-flow` nếu không sửa thư viện. v1: trước khi sync, nếu có job ingest Knowledge đang chạy thì hoãn (`SyncStatus::Deferred { reason: IngestRunning }`) và thử lại sau 30 s; tối đa hoãn 10 phút thì vẫn sync và ghi cảnh báo vào log. Ticket cần tìm (hoặc thêm) truy vấn "có job ingest đang chạy".
- **Lịch chạy** như desktop: khi server khởi động, debounce sau khi có ghi (middleware ghi nhận thời điểm), và định kỳ.
- **Chỉ lock holder sync.** Nếu app desktop trên cùng máy đang giữ `flock` của vault thì server ở chế độ reader và sync báo `Disabled { ReadOnly }`.

### Cấu hình `config.toml`

```toml
[wiki.sync]
enabled = false
remote_url = "git@github.com:<owner>/<repo>.git"   # SSH; URL có credential bị từ chối
branch = "main"
ssh_key_path = "/app/docker/wiki-deploy-key"       # mount read-only, quyền 0600
known_hosts_path = "/app/docker/github-known-hosts"
author_name = "Vela Server"
author_email = "vela@localhost"
interval_min = 5
debounce_sec = 30
max_file_mb = 50
```

`config/validation.rs` kiểm tra: `remote_url` không chứa credential; khi `enabled` thì `ssh_key_path` và `known_hosts_path` phải tồn tại; khóa không được để quyền đọc cho group/other. Remote **không** sửa được từ Admin Web: nếu sửa được, ai có quyền admin cũng có thể trỏ vault tới repo của mình để lấy dữ liệu. Muốn đổi thì sửa `config.toml` và khởi động lại.

### Docker

- `server.Dockerfile` (stage `runtime`): thêm `git openssh-client`.
- `compose.prod.yaml`: mount `./docker/wiki-deploy-key` và `./docker/github-known-hosts` read-only, `create_host_path: false`, giống `credential-keys.json`.
- `docker/config.prod.example.toml` và `config.example.toml`: thêm mục `[wiki.sync]` với `enabled = false`.
- Hướng dẫn tạo deploy key (`ssh-keygen -t ed25519`, thêm vào GitHub → Settings → Deploy keys, bật "Allow write access") và lấy `known_hosts` (`ssh-keyscan github.com`, đối chiếu fingerprint GitHub công bố) ghi vào `docs/knowledge/`.

### HTTP (admin, cần token admin như các route Knowledge khác)

| Route | Việc |
|---|---|
| `GET /api/admin/knowledge/sync` | `SyncStatus` + cấu hình đã che (remote, branch, interval; không có đường dẫn khóa) |
| `POST /api/admin/knowledge/sync` | `sync_now`; trả `SyncReport`. Đang sync thì trả `409 sync_in_progress` |

SSE `GET /knowledge/events` phát thêm event `sync_status` mỗi khi `SyncStatus` đổi. Lỗi trả theo kiểu lỗi admin đang dùng (`{ error: "sync_auth" }`, `sync_unsafe_remote`, `sync_disabled`, …), không chứa stderr thô.

## UI (dùng chung cho desktop và Admin Web)

Đặt trong `packages/knowledge-ui`:

```ts
export interface SyncBackend {
  readonly configurable: boolean          // true ở desktop, false ở Admin Web
  status(): Promise<SyncStatus>
  syncNow(): Promise<SyncReport>
  configure?(input: SyncSettingsInput): Promise<SyncStatus>  // chỉ desktop
  disable?(): Promise<void>                                   // chỉ desktop
  onStatus(listener: (status: SyncStatus) => void): () => void
}
```

`KnowledgeBackend`/`AdminKnowledgeBackend` thêm thuộc tính tùy chọn `sync?: SyncBackend`; không có thì UI ẩn mọi phần sync. Adapter Tauri gọi command và event `sync://status`; adapter admin gọi hai route trên và nghe `sync_status` trên SSE sẵn có.

- Component `KnowledgeSyncPanel`:
  - `configurable = true`: remote URL, branch, bật/tắt tự động, khoảng thời gian, "Sync now", "Tắt sync". Desktop mở nó trong Settings.
  - `configurable = false`: chỉ đọc remote/branch/lịch, nút "Sync now", và dòng "Cấu hình trong `config.toml` mục `[wiki.sync]`". Admin Web mở nó từ huy hiệu trên trang Knowledge.
- Huy hiệu trạng thái (`Đã đồng bộ 2 phút trước`, `↑3 ↓1`, `Đang đồng bộ…`, `Đang chờ job ingest`, `2 xung đột`, `Lỗi xác thực`): footer của desktop, thanh công cụ trang Knowledge của Admin Web. Bấm vào mở panel.
- `Conflicted`: liệt kê bản conflict, bấm để mở trang.
- `Disabled { ReadOnly }`: "Process khác đang giữ vault, sync tạm tắt".
- Chuỗi hiển thị đi qua `i18n.ts` của package.

## Cách test

Chạy bằng `scripts/agent-test.sh`. Test dùng một bare repo trong `tempdir` làm "GitHub" và hai `Vault` trên hai clone làm hai máy. Không gọi mạng.

- Ghi ở A, sync A, sync B → B `read`/`search` thấy nội dung mới, nhận `External`.
- Cùng sửa một trang ở A và B → B có file `(conflict …)`, trang gốc giữ bản của B, `status` là `Conflicted`.
- Cả hai bên thêm dòng vào `wiki/log.md` → sau sync có đủ dòng của cả hai.
- `wiki/index.md` xung đột → được sinh lại, không còn conflict marker.
- `.wiki/index.sqlite` không bao giờ có trong `git ls-files`.
- File lớn hơn `max_file_bytes` → `FileTooLarge`, không có commit mới.
- URL có credential → `UnsafeRemote`.
- Vault mở ở chế độ reader → `sync_now` trả `ReadOnly`.
- `PATH` không có git → `status` là `Disabled { NoGit }`.

Test đánh dấu bỏ qua khi máy chạy test không có git, nhưng in rõ là `SKIPPED`.

## Ngoài phạm vi v1

- Sửa cấu hình sync của server từ Admin Web.
- Chặn từng lần ghi bên trong `orchestrator-flow` (cần sửa thư viện).
- Git LFS cho `sources/assets/`.
- PAT trong Keychain.
- Giao diện gộp xung đột trong app, lịch sử phiên bản, khôi phục từ commit cũ.
