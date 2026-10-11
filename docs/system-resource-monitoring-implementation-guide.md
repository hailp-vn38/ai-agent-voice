# System Resource Monitoring — Hướng dẫn triển khai (Ponytail Revision)

> **Repository:** [hailp-vn38/ai-agent-voice](https://github.com/hailp-vn38/ai-agent-voice)\
> **Đối chiếu code:** nhánh `main`, 2026-10-11\
> **Trạng thái:** đã triển khai sampler, API và giao diện; kiểm chứng tự động được ghi ở cuối tài liệu.\
> **Quyết định:** giám sát snapshot hiện tại; không có CPU/Memory Usage History; giữ nguyên cấu hình Studio Assistant / Floating Chat.

## 1. Phạm vi và kết quả mong muốn

Thêm **một** endpoint tài nguyên hệ thống và bốn card giám sát vào trang `/system`. Server đọc CPU/RAM, process CPU/RSS, dung lượng root filesystem mỗi **5 giây** bằng sampler dùng chung; web đọc snapshot đã thu thập, không lấy mẫu OS trong HTTP handler.

**Có:**

- CPU hệ thống, RAM hệ thống, CPU/RSS của tiến trình Rust, dung lượng root filesystem.
- Status, uptime, version, database, sessions và provider runtime **tái sử dụng** từ `GET /api/admin/system`.
- Giao diện responsive, progress bar, `loading` / `unavailable` / `stale` và refresh thủ công.
- Bảo toàn toàn bộ form và flow Virtual Device hiện hữu.

**Không làm:** CPU Usage History, Memory Usage History, time-series chart, metrics database, cgroup quota/container metrics, GPU, network throughput, SSE/WebSocket metrics, alert thresholds hoặc thay đổi Voice Session pipeline. Không tạo `resources/history`.

> **Ngữ nghĩa hiển thị:** `sysinfo` cho các chỉ số **system / OS-visible**. Trong Docker, các giá trị này có thể phản ánh host thay vì giới hạn container. UI dùng nhãn “System CPU / System RAM (OS visible)”, **không** quảng bá là mức sử dụng container. Container-aware cgroup v2 có thể là tài liệu/phase riêng khi có yêu cầu thực tế.

## 2. Codebase và các seam cần tái sử dụng

| Vị trí | Hành động |
| --- | --- |
| `crates/voice-agent-server/src/app/admin/system.rs` | Giữ `get_system`; thêm handler đọc snapshot. |
| `crates/voice-agent-server/src/app/admin/mod.rs` | Mount route mới **dưới middleware bearer auth hiện tại**. |
| `crates/voice-agent-server/src/app/state.rs` | `AppState` giữ `Arc<SystemMetrics>`; mọi clone dùng chung dữ liệu. |
| `crates/voice-agent-server/src/app/mod.rs` | Bắt đầu sampler ở production bootstrap **sau khi có AppState**, trước khi trả router; cả nhánh managed và non-managed. |
| `crates/voice-agent-server/src/lifecycle.rs` | Dùng token `listening()` để dừng sampler khi shutdown bắt đầu. |
| `crates/voice-agent-server/src/services/provider_runtime/status.rs` | Đã có `RuntimeAccounting.reserved_bytes`, `active_leases` và các counter khác. |
| `apps/admin-web/src/api/client.ts` | Tái sử dụng `requestJson`, Bearer token, `AbortSignal`. |
| `apps/admin-web/src/stores/server.ts` | Tái sử dụng `status`, `refreshStatus()`, `checkHealth()`; **không** đưa polling 5s vào store này. |
| `apps/admin-web/src/views/SystemView.vue` | Thay bố cục card cũ, quản lý trạng thái resources **cục bộ tại page**. |
| `apps/admin-web/src/components/system/AssistantDeviceSettings.vue` | **Giữ nguyên implementation, chỉ giữ vị trí render trong SystemView.** |

Contract cần bảo vệ: [Studio Assistant API](https://github.com/hailp-vn38/ai-agent-voice/blob/main/docs/api/studio-assistant.md), [Admin Web API](https://github.com/hailp-vn38/ai-agent-voice/blob/main/apps/admin-web/docs/api.md).

### 2.1. Không nhầm số liệu memory

- `process.rss_bytes`: RAM vật lý tiến trình (OS đo), có thể đã bao gồm RAM các provider đã nạp.
- `provider_runtime.accounting.reserved_bytes`: **ước lượng reservation**, không phải RAM đang dùng.
- `system.memory_used_bytes`: RAM của môi trường mà OS báo cáo, có thể bao gồm workload khác.
- **Không cộng** RSS với `reserved_bytes` hoặc RAM system để tạo tổng RAM.

## 3. Thiết kế Rust tối giản

### 3.1. Một deep module, một interface đọc

**File mới:** `crates/voice-agent-server/src/services/system_metrics.rs` (không tạo thư mục nhiều file, trait collector hay lớp adapter khi chỉ có một implementation).

```text
SystemMetrics (owned by AppState)
   ├── sysinfo system/process/disk sampler (background, 5s)
   ├── latest immutable ResourceSnapshot
   └── latest()  ────────────> GET /api/admin/system/resources
                                      |
                                SystemView.vue

GET /api/admin/system ────────> useServerStore ──> AI Runtime / Server info
AssistantDeviceSettings ─────> existing assistantApi / WebSocket (unchanged)
```

**Interface bên ngoài** (pseudocode, có thể đổi chi tiết Rust cho phù hợp implementation):

```rust
pub struct SystemMetrics { /* private snapshot + sampler data */ }

impl SystemMetrics {
    pub fn new() -> Self;
    pub fn latest(&self) -> Option<ResourceSnapshot>; // None nếu chưa có / đã quá hạn
}
```

- `ResourceSnapshot` là struct `Serialize + Clone` chứa **duy nhất dữ liệu OS** (không có field `runtime`).
- Thời gian lấy mẫu UTC `sampled_at` để hiển thị; dùng `Instant` nội bộ để phát hiện dữ liệu quá **15 giây** mà không phụ thuộc clock hệ thống.
- Store một snapshot dưới `RwLock<Option<...>>` hoặc cơ chế tương đương có lock ngắn; thu thập OS **ngoài lock** và publish atomically.
- Không có history, queue, retry scheduler, `OnceLock` hay atomic `start_once` guard. Không bổ sung public traits để phục vụ một sampler.
- Dùng `sysinfo` (phiên bản tương thích Rust toolchain của repo) để đọc system memory/CPU, current PID/process RSS/CPU, disks. Chỉ refresh phạm vi cần thiết; tránh quét mọi process.
- `% CPU` cần baseline giữa các lần lấy mẫu: mẫu đầu có thể `null` thay vì số 0 giả. Process CPU là **raw core-based percent**, có thể vượt 100% trên nhiều core; chỉ hiển thị dạng text, **không** dùng raw CPU này cho progress bar.
- Tất cả bytes truyền dạng integer bytes; frontend chuyển MiB/GiB. Giá trị không lấy được là `null`, không tự biến thành `0`.

### 3.2. Vòng sampling và lifecycle

**Quyết định ownership:** `AppState` khởi tạo `Arc<SystemMetrics>` trong constructor chung. Production bootstrap tạo **một** background sampling task cho state đó, ngay trước `router_with_state(state)` ở nhánh managed và non-managed. Không tự spawn trong `AppState::clone`, `router_with_state()` hay từng HTTP request; vì các hàm này có thể được gọi nhiều lần bởi test.

- Trình tự mỗi tick: lấy mẫu bằng `tokio::task::spawn_blocking` nếu thao tác `sysinfo` blocking → publish snapshot → đợi tick kế tiếp; **await xong** lần đo trước rồi mới chạy lần mới, không chồng nhiều task.
- Một sampler sở hữu `sysinfo` state qua các tick để giữ baseline CPU. Khi cần chia task `spawn_blocking`, chuyển quyền sở hữu state vào closure rồi lấy lại, không tạo `sysinfo` mới mỗi tick.
- Dừng khi `lifecycle.listening()` cancelled. Không đợi thêm một vòng 5 giây để shutdown; không gắn phần sampler vào voice drain/grace.
- Monitoring lỗi/OS inaccessible không làm hỏng `/health`, `/ready` hoặc startup của voice server. Có thể log một lỗi ngắn gọn, **không** log host paths hay secrets.
- Constructor dành cho các test dựng `router_with_state` trực tiếp vẫn có snapshot store nhưng **không tự mở sampler**. HTTP test muốn có dữ liệu thì khởi chạy sampler trong fixture và đợi snapshot sẵn sàng với timeout. Có thể dùng fixture dữ liệu nội bộ trong unit tests, không mở rộng public interface chỉ để test.

**Không thêm một tầng lifecycle manager mới**; chỉ reuse cancellation token hiện hành. Không bắt buộc support stop-on-drop riêng cho từng AppState clone trong đợt này vì production lifecycle đã có owner và shutdown rõ ràng.

### 3.3. Disk: duy nhất root filesystem

Đo filesystem tương ứng mount `/` trong namespace mà tiến trình nhìn thấy bằng thông tin disk của `sysinfo`; không quét/cộng dung lượng các mount khác, không nhận data directory/path tùy biến. Nếu không tìm được mount đáng tin cậy, trả storage fields `null` và UI hiển thị `Unavailable`.

Nhãn UI: **Root filesystem**; không gọi là “Model Storage” hoặc tự suy ra là ổ chứa model. Không trả filesystem/mount path trong response.

## 4. HTTP contract

### 4.1. Endpoint và lỗi

```http
GET /api/admin/system/resources
Authorization: Bearer <admin-token>
```

- `200 OK` + `Cache-Control: no-store`: snapshot còn mới. Một số field có thể `null` khi OS không hỗ trợ.
- `503` + existing Admin error envelope, code **`system_metrics_unavailable`**: chưa có snapshot hoặc snapshot quá 15 giây. **Không** tạo error code `system_metrics_stale` riêng.
- `401` khi thiếu/sai Bearer; endpoint chỉ mount nếu `api.enabled=true`, cùng `admin::router` hiện có.
- Không gọi SQLite, `ProviderRuntimeManager`, filesystem sampling hoặc xử lý blocking từ HTTP handler.
- `GET /health`, `GET /ready`, `GET /api/admin/system` **giữ nguyên contract**. Khi database lỗi, resources vẫn có thể trả snapshot OS; Overview vẫn xử lý `system` như trước.

### 4.2. DTO duy nhất (wire contract nguồn chuẩn)

Định nghĩa `SystemResources` trong `apps/admin-web/src/api/types/system.ts`; Rust `ResourceSnapshot` dùng cùng field/nullable semantics. **Không duy trì một bản JSON ví dụ đầy đủ trùng DTO**.

```ts
export interface SystemResources {
  sampled_at: string // UTC ISO-8601
  sample_interval_ms: number // 5000
  system: {
    cpu_percent: number | null // 0..100; null trước baseline
    logical_cores: number | null
    memory_total_bytes: number | null
    memory_used_bytes: number | null
  }
  process: {
    pid: number
    cpu_percent: number | null // raw core-based, có thể >100
    rss_bytes: number | null
  }
  storage: {
    total_bytes: number | null // root filesystem
    available_bytes: number | null
  }
}
```

- `system` có nghĩa **OS-visible system**, không phải container memory quota hay host-verified memory.
- `memory_total_bytes >= memory_used_bytes` khi cùng snapshot có số liệu hợp lệ. Nếu `memory_total_bytes=0` thì UI không tính phần trăm.
- `storage.used_bytes = max(0, total_bytes - available_bytes)` **tính ở frontend** để tránh thêm field dư. Phần trăm chỉ hợp lệ khi `total_bytes > 0` và tất cả values hợp lệ; clamp gauge về `[0,100]` để không tràn layout. Không giả định disk available là dung lượng container.
- `sample_interval_ms` là tần suất cấu hình; timestamp phản ánh lần đo cuối, không phải lúc HTTP response được tạo.
- Server không trả OS paths, DB paths, cgroup paths, token, provider configs, user identity hoặc command line.

### 4.3. Handler chỉ đọc snapshot

```rust
pub(super) async fn get_resources(State(state): State<AppState>, request: Request) -> Response {
    let Some(snapshot) = state.system_metrics.latest() else {
        return error(&request, StatusCode::SERVICE_UNAVAILABLE, "system_metrics_unavailable");
    };
    // Serialize typed snapshot và set Cache-Control: no-store.
    response_no_store(snapshot)
}
```

Đây là pseudocode, không phải patch copy-paste; dùng helpers response/error của Admin router và giữ request ID / headers nhất quán. Logic quá hạn 15 giây nằm **trong `latest()`**, không lặp ở handler/web.

## 5. Admin Web — thiết kế System page

### 5.1. Bố cục

```text
System                                                [Refresh]
Online · Version · Uptime · Last updated

┌────────────┬────────────┬──────────────┬──────────────┐
│ System CPU │ System RAM │ Rust Process │ Root Disk    │
│ 34.5%      │ 10/16 GiB  │ RSS 2.1 GiB  │ 61.2%        │
│ progress   │ progress   │ CPU 42.8%    │ progress     │
└────────────┴────────────┴──────────────┴──────────────┘

AI Runtime
Sessions · Providers loaded/failed/stale · Reserved estimate · Active leases

Studio Assistant Settings  ← GIỮ NGUYÊN AssistantDeviceSettings

Server Information / Voice Transport
Rust/Axum · PID · Database · OTA · WS · Vision
```

Số liệu chỉ minh họa. Thứ tự render: `PageHeader` → resource cards → runtime → `AssistantDeviceSettings` → server/transport.

- **Header:** tái sử dụng `PageHeader` và `useServerStore`; health từ `/health`, version/uptime/runtime từ `/api/admin/system` (refresh khoảng 30 giây).
- **Bốn cards:** `ResourceStatCard.vue` tái sử dụng UI primitive có sẵn. Progress chỉ cho system CPU, RAM và Disk; process hiển thị RSS + raw CPU text (không gauge CPU đa core sai nghĩa).
- **AI Runtime:** render **trực tiếp trong `SystemView.vue`**, đọc `server.status.sessions.active`, `providers.loaded/failed/stale`, `provider_runtime.accounting.reserved_bytes`, `active_leases` (nếu có); không tạo `RuntimeSummary.vue` hoặc duplicate runtime field trong resources endpoint. Có nhãn **Estimated reservation**, không cộng với RSS.
- **Studio Assistant:** render `AssistantDeviceSettings.vue` **đúng một lần**, không thêm dynamic `key` phụ thuộc sampler; không sửa logic dirty/revision, save/disable confirmation, `assistant.open()` và WS ticket. Không remount khi metrics refresh.
- **Server Information / Transport:** giữ thông tin `Rust/Axum`, database, PID, URL routes OTA/WS/Vision (như UI hiện tại); gộp/thu gọn cards tĩnh cũ. Có thể chuyển Management Data sang Overview hoặc hiển thị nhỏ là “cache-loaded”, không dùng làm nguồn tổng DB thực tế.
- **API Boundary:** bỏ card mô tả tĩnh nếu trùng các thông tin trên; giữ action `admin.refreshAll()` chỉ khi có nhãn riêng “Reload management data”. Không gộp vào Refresh của monitoring.
- **Responsive:** `grid-cols-1 sm:grid-cols-2 xl:grid-cols-4`, design tokens / dark-light hiện hữu, no horizontal overflow, keyboard và a11y.

### 5.2. Chỉ dùng local state của SystemView

Không tạo `useSystemMetricsStore`. Dùng `ref<SystemResources|null>` và `loading/error` trong `<script setup>` của `SystemView.vue`. Tách helper tính format/percentage nếu cần nhưng không tạo composable chung cho một page.

**Lifecycle polling:**

1. `onMounted`: gọi health + status thông qua `server.refresh()`, đọc resources ngay. `AssistantDeviceSettings` tự load như cũ.
2. Mỗi **5 giây**, chỉ gọi `systemApi.resources()` khi tab đang visible và page còn mounted. Nếu một request đang chạy thì **skip** tick đó; không concurrent fetch.
3. Health/status vẫn refresh ở cadence **30 giây**, không query `/api/admin/system` mỗi 5 giây vì endpoint này có DB/provider overview.
4. Khi hidden → visible: gọi refresh resources ngay, đồng thời không lập trùng interval. Khi `onBeforeUnmount`, clear timers, remove visibility listener và abort pending fetch.
5. Khi thành công, hiển thị thời gian từ `sampled_at`; khi response có timestamp quá **15 giây**, UI hiện `Stale`/`—` như guard hiển thị, **không tự phát minh mã lỗi HTTP mới**.
6. `401` riêng cho resources không đồng nghĩa `/health` offline; `503` không reset server status hoặc assistant form. `null` hiển thị `—`, không dùng `?? 0`.
7. Refresh thủ công gọi health + status + resources, không gọi `assistantApi.save`, `assistantApi.ticket` hoặc reset unsaved settings; `admin.refreshAll()` vẫn là action khác.

**Race handling tối thiểu:** một `AbortController` cho resources theo vòng đời page, một `inFlight` guard để skip overlap; hủy controller ở unmount. Với button refresh khi đang in-flight, có thể skip hoặc disable nút tạm thời; không cần request generation counters hoặc state machine phức tạp. Không sử dụng cùng AbortController đã aborted cho lần mount khác.

```ts
// apps/admin-web/src/api/system.ts
resources(signal?: AbortSignal) {
  return requestJson<SystemResources>('/api/admin/system/resources', {}, { signal })
}
```

`systemApi.resources()` dùng đúng `requestJson` đã đính kèm Admin bearer. Không tự viết `fetch`, không tạo auth flow hoặc websocket mới.

## 6. File cần thay đổi

**Rust**

```text
Cargo.toml                              # sysinfo workspace dependency (nếu project convention)
Cargo.lock
crates/voice-agent-server/Cargo.toml
crates/voice-agent-server/src/services/mod.rs
crates/voice-agent-server/src/services/system_metrics.rs     # NEW: collector + snapshot
crates/voice-agent-server/src/app/state.rs
crates/voice-agent-server/src/app/mod.rs
crates/voice-agent-server/src/app/admin/mod.rs
crates/voice-agent-server/src/app/admin/system.rs
crates/voice-agent-server/tests/admin_api.rs
```

**Admin Web**

```text
apps/admin-web/src/api/system.ts
apps/admin-web/src/api/types/system.ts
apps/admin-web/src/components/system/ResourceStatCard.vue  # NEW
apps/admin-web/src/views/SystemView.vue
apps/admin-web/src/i18n/messages.ts                          # EN + VI
apps/admin-web/src/api/resources.test.ts
```

**Docs và API Collection:** `docs/api/00-all-apis.postman_collection.json`, `apps/admin-web/docs/api.md`, `apps/admin-web/docs/workflows.md`, `apps/admin-web/README.md`.

**Không chỉnh sửa:** `AssistantDeviceSettings.vue`, `api/assistant.ts`, `stores/assistant.ts`, `lib/assistant/*`, các WS/Virtual Device routes hoặc voice pipeline. Nếu phát hiện lỗi độc lập, tách patch/PR riêng.

## 7. Test + triển khai: một checklist

- [x] Thêm dependency `sysinfo` theo workspace convention; lockfile cập nhật; giữ toolchain Rust 1.98 tương thích.
- [x] Viết `SystemMetrics` có **một** snapshot, một sampler production/State, `latest()` trả `None` khi chưa ready hoặc quá 15s; CPU trước baseline là `null`.
- [x] Startup nhánh managed/non-managed khởi động sampler một lần; `AppState::clone` và `router_with_state` không spawn; sampler dừng theo `lifecycle.listening()`.
- [x] OS reads chạy ngoài Tokio reactor/ngoài snapshot lock; process CPU raw có thể >100; disk chỉ root filesystem, missing fields `null`.
- [x] Route GET resources bảo vệ bởi Admin bearer, `200` JSON + `Cache-Control: no-store`, `401` unauthorized, `503 system_metrics_unavailable`; `api.enabled=false` không mount route.
- [x] `GET /api/admin/system` cùng `/health`, `/ready` không thay đổi; DB unavailable không ngăn metrics trả OS snapshot (nếu collector còn hoạt động).
- [x] Frontend `SystemResources` nullable đúng wire shape, `systemApi.resources()` reuse request wrapper và `AbortSignal`.
- [x] SystemView có đúng bốn cards, giá trị bytes định dạng GiB/MiB, progress clamp `[0,100]`, null/zero-total không gây NaN.
- [x] Web chỉ poll resources 5s ở `/system`, pause hidden, resume visible, skip in-flight, abort on unmount. Server status refresh 30s, không 5s.
- [x] AI Runtime chỉ đọc `server.status` + `provider_runtime.accounting`; reservation có nhãn estimate, không cộng với RSS.
- [x] `AssistantDeviceSettings` không bị remount khi poll, không mất giá trị đang edit; Save/Open Chat/confirm disable và ticket/WS vẫn chạy.
- [x] Test endpoint auth/availability/no-store, collector baseline/expiry/partial failure, web polling/visibility/unmount, `null`/stale, assistant unsaved form regression.
- [x] Update Postman + API docs + workflows + web README; chạy lệnh kiểm tra dưới đây và ghi lại test thực tế đã chạy.

```bash
cargo fmt --all --check
cargo check -p voice-agent-server
cargo test -p voice-agent-server --test admin_api

cd apps/admin-web
npm test
npm run typecheck
npm run build
```

## 8. Các quyết định không triển khai ở phase này

- **Không cgroup v2/quota normalized CPU:** chưa có nhu cầu hiển thị container utilization đáng tin cậy; `sysinfo` OS-visible đủ cho dashboard hiện tại.
- **Không store/Composable monitoring toàn cục:** chỉ SystemView cần poll; `useServerStore` hiện giữ health/aggregate riêng.
- **Không nhân bản `active_sessions` hoặc `reserved_bytes`:** `/api/admin/system` đã cung cấp; resources endpoint chỉ làm một việc là OS snapshot.
- **Không đa module collector / extra adapter traits:** chỉ có một OS sampler, một page consumer; sẽ tách khi thay đổi thực sự đòi hỏi.
- **Không SSE/WS metrics, Prometheus hoặc SQL metrics history:** polling HTTP 5 giây đáp ứng use case.
- **Không refactor Studio Assistant:** đây là module cấu hình có revision và admission/WS flow riêng.

---

**Hướng dẫn implementation agent:** đọc lại code `main` trước khi sửa; giữ thay đổi theo danh sách nhỏ trên. Nếu môi trường không hỗ trợ một metric thì trả `null`/Unavailable có lý do hiển thị, không phát sinh fallback số giả. Nếu không chạy được test hoặc Docker smoke, ghi rõ chưa kiểm chứng; phân biệt kiểm chứng tự động với smoke trên môi trường thật.


## 9. Ghi nhận triển khai — 2026-10-11

- Sampler được khởi động tại hai nhánh production startup; constructor/clone/router không spawn. Fixture HTTP khởi động sampler và đợi snapshot với timeout.
- `sysinfo` giữ CPU baseline giữa các tick. Disk state được tạo rỗng mỗi lần đo vì refresh lỗi có thể giữ capacity cũ; chỉ refresh storage của mount root. Linux kiểm tra khả năng đọc nguồn CPU/RAM trước khi công bố để không đóng dấu thời gian mới lên số liệu giữ lại khi nguồn OS không đọc được. CPU process có baseline riêng, cần một lần đo thành công trước khi trả phần trăm.
- Test web dùng component Assistant thật để kiểm tra form chưa lưu giữ nguyên DOM và giá trị qua polling/Refresh, không gọi save/ticket hoặc reload settings.
- Đã pass: `cargo fmt --all --check`, `cargo check -p voice-agent-server`, collector tests, HTTP resources test; `npm test` (47 files, 185 tests), `npm run typecheck`, `npm run build`.
- Commit gate đã pass trên source cuối: `cargo clippy --all-targets --all-features -- -D warnings`; `cargo test --workspace` (55 targets, 651 passed, 6 ignored). Lệnh `cargo test -p voice-agent-server --test admin_api` cũng pass đủ 39 tests. Sáu test ignored giữ nguyên theo cấu hình suite hiện hữu.
- Chưa chạy Docker/browser smoke với provider thật hoặc hardware. Không có metrics container/cgroup. Trên OS ngoài Linux, sysinfo không báo lỗi refresh CPU/RAM chi tiết; cần thêm probe nền tảng nếu triển khai ở đó cần đảm bảo phát hiện lỗi nguồn OS giữa các lần đo.


### Review Standards

Không có finding cần xử lý. Route dùng Admin auth hiện hành, snapshot không trả paths/secrets, sampler theo listening cancellation, tests nằm riêng theo CONTEXT.md. Không thêm abstraction hoặc thay đổi voice pipeline.

### Review Spec

Đã sửa hai finding: disk refresh lỗi không giữ capacity cũ dưới timestamp mới; CPU process dùng baseline riêng và trả null khi chưa có lần đo thành công trước đó. Re-review không còn finding cụ thể chưa xử lý. Giới hạn phát hiện lỗi refresh trên OS ngoài Linux đã được ghi rõ ở trên.
