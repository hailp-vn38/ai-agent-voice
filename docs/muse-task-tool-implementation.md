# Tài liệu triển khai tool `muse_task`

Trạng thái: sẵn sàng triển khai, chưa có code. Cập nhật 2026-11-01.
Lý do và các phương án đã cân nhắc nằm ở [tài liệu thiết kế](muse-task-tool-design.md). Tài liệu này chỉ nói làm gì, ở đâu và theo thứ tự nào.

Đường dẫn `crates/…` tính từ gốc repo. Đường dẫn `orchestrator_agent_rs/…` nằm trong `../libs/`, bản 0.6.0. Không sửa thư viện trong đợt này.

## Hành vi cần đạt

| # | Hành vi |
| --- | --- |
| H1 | Mọi lần gọi `muse_task` trong một subagent dùng chung một Muse session; hai subagent dùng hai session |
| H2 | Root gọi tool nhận `MUSE_TASK_SUBAGENT_ONLY` |
| H3 | Subagent xong thì Muse session bị xóa và subagent bị archive, trong khoảng 10 giây |
| H4 | Kết quả gồm message và file; `output=markdown` luôn cho ra một artifact `.md` |
| H5 | Việc dài hơn một lát chờ không gửi lại prompt, kể cả sau khi restart |
| H6 | Đứt WebSocket được nối lại ngay, không ngồi chờ tới hết hạn |
| H7 | Session mất giữa chừng báo `MUSE_SESSION_LOST`, không tự tạo session mới |

## Các module

Có bốn module sâu. Tool mỏng, phần lớn hành vi nằm sau `AgentTaskProvider`.

```text
Subagent
   │  muse_task(args)
   ▼
MuseTaskTool ──────────────► AgentTaskProvider            ← seam mới
   │ artifact, journal,          │ adapter: LlmGatewayProvider, FakeAgentTaskProvider
   │ định dạng kết quả           ▼
   │                         agent_task::run_turn
   │                             ├─ TurnTracker           ← không I/O
   │                             ├─ file_paths            ← không I/O
   │                             └─ MuseVmTransport       ← seam có sẵn, đã có VM giả
   ▼
Orchestrator ◄──── SubagentMuseCloser (vòng 10 giây) ───► release_recorded_job_session
```

| Module | Vị trí mới | Phụ thuộc | Cách test |
| --- | --- | --- | --- |
| `MuseTaskTool` | `services/task_flow/muse_task/tool.rs` | Port `AgentTaskProvider`, `ToolContext` | Orchestrator thật trên SQLite tạm, provider giả |
| `agent_task::run_turn` | `providers/llm/gateway/muse/workflow/agent_task/mod.rs` | `MuseVmTransport`, `OperationJournal` | VM giả qua `build_gateway_muse_provider` |
| `TurnTracker`, `file_paths`, `prompt` | `…/workflow/agent_task/{tracker,files,prompt}.rs` | Không | Test thuần, truyền thời gian vào |
| `SubagentMuseCloser` | `services/task_flow/muse_task/closer.rs` | Orchestrator, job session registry | `decide_closure` thuần; một test tích hợp với VM giả |

`ToolContext::new` là `pub(crate)` trong thư viện, nên test tool phải chạy qua Orchestrator, theo mẫu `services/task_flow/tests_memory.rs`.

## Interface

### `AgentTaskProvider`

Thêm vào `crates/voice-agent-server/src/providers/llm/mod.rs`, cạnh `DurableMediaProvider`. Seam này tồn tại vì tool chỉ cầm `Box<dyn LlmProvider>`, và vì có hai adapter thật: gateway và bản giả cho test tool.

```rust
pub const MUSE_TASK_KEY_PREFIX: &str = "muse_task:";

pub enum AgentTurnAction {
    Send { prompt: String, output: AgentOutputKind, filename: Option<String> },
    Collect { output: AgentOutputKind, filename: Option<String> },
}

pub struct AgentTurnRequest {
    pub session_key: String,                       // "muse_task:<session_id của subagent>"
    pub action: AgentTurnAction,
    pub operation: orchestrator_agent_rs::ToolOperation, // deadline_at là trần tuyệt đối
    pub wait_slice: Duration,                      // hết lát thì trả RetryLater
}

pub struct AgentTurnOutput {
    pub turn: u32,
    pub message: String,
    pub files: Vec<AgentFile>,
    pub files_missing: bool,                       // output=files mà Muse không đính kèm
}

pub struct AgentFile { pub filename: String, pub mime_type: String, pub bytes: Vec<u8>, pub source_path: String }

#[async_trait::async_trait]
pub trait AgentTaskProvider: Send + Sync {
    async fn run_turn(
        &self,
        request: &AgentTurnRequest,
        journal: &dyn OperationJournal,
        progress: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<AgentTurnOutput, AgentTaskError>;
}
```

Thêm `fn agent_task(&self) -> Option<&dyn AgentTaskProvider> { None }` vào `LlmProvider`, theo mẫu `durable_media`.

Điều bên gọi phải biết:

- `run_turn` tự quyết định gửi, đối soát, chờ hay tải lại dựa trên `operation.status`. Khác với `DurableMediaProvider`, tool không phân nhánh theo trạng thái. Trạng thái kết thúc (`Committed`, `Failed`, `Expired`, `Cancelled`) tool xử lý trước khi gọi.
- Gọi lại cùng invocation là an toàn và không gửi trùng prompt.
- Không hàm nào ở đây xóa session. Chỉ `SubagentMuseCloser` được xóa.

```rust
pub enum AgentTaskError {
    RetryLater(String),                  // → TOOL_RETRY_LATER
    OutcomeUnknown,                      // → TOOL_OUTCOME_UNKNOWN
    Busy,                                // → MUSE_TASK_BUSY
    NoSession,                           // → MUSE_TASK_NO_SESSION (collect khi chưa send)
    SessionLost,                         // → MUSE_SESSION_LOST
    Stalled { partial: Option<String> }, // → MUSE_TASK_STALLED
    Timeout { partial: Option<String> }, // → MUSE_TASK_TIMEOUT
    NeedsReauth,                         // → PROVIDER_AUTH_FAILED
    Rejected(String),                    // → MUSE_TASK_REJECTED
}
```

Lỗi tạm thời và quota đều thành `RetryLater` nếu đã có lượt gửi, vì session chỉ dùng được trên tài khoản đã ghim. Chưa gửi gì mà lỗi thì thành `Rejected`, để không tốn lượt retry.

### Tham số tool

Như [tài liệu thiết kế](muse-task-tool-design.md#giao-diện): `action`, `prompt`, `output`, `filename`, `max_wait_secs`. Descriptor dùng `ToolEffect::Resumable`, `timeout()` trả 1800s.

| Hằng số | Giá trị | Vị trí |
| --- | --- | --- |
| `TOOL_TIMEOUT` | 1800s | `muse_task/tool.rs` |
| `WAIT_SLICE` | 1700s | `muse_task/tool.rs` |
| `DEFAULT_MAX_WAIT_SECS`, khoảng cho phép | 3600s, 60 đến 7200s | `muse_task/tool.rs` |
| `STALL_AFTER` | 600s | `agent_task/mod.rs` |
| `RECONNECT_GIVE_UP` | 300s; backoff 1, 2, 5, 10, 30s | `agent_task/mod.rs` |
| `SLOT_WAIT` | 60s | `agent_task/mod.rs` |
| `CLOSER_INTERVAL` | 10s | `muse_task/closer.rs` |

## Luồng trong `run_turn`

```text
acquire lease theo account đã ghim, giới hạn SLOT_WAIT; lease.retain_session()
khóa gate của job_entry(session_key)

theo action và operation.status:
  Send + Submitting chưa có baseline → kiểm tra Busy và SessionLost
                                     → journal {account, session, baseline, submitting}
                                     → /chat/stream, journal {running, session thật}
  Send + Submitting có baseline      → đếm lượt user: > baseline thì Running;
                                       = baseline thì gửi; không đọc được thì OutcomeUnknown
  Running                            → chờ
  Downloading                        → tải lại
  Collect                            → không gửi, chờ lượt gần nhất

vòng chờ (TurnTracker)
  sự kiện, hoặc history theo nhịp 2s → 5s → 15s
  lỗi transport → lease.reconnect(), backoff; quá RECONNECT_GIVE_UP → RetryLater
  hết wait_slice, Muse còn chạy → RetryLater
  quá deadline_at → Timeout{partial}; không có tín hiệu quá STALL_AFTER → Stalled{partial}

Done → journal downloading → đọc file → trả AgentTurnOutput (lease.finish_clean(None))
```

Bốn điểm dễ sai:

1. **Không gọi `fail_account` với lỗi tạm thời.** Hàm đó vẫn đóng WebSocket dùng chung của tài khoản (G2). Với `AuthInvalid` và `QuotaExceeded` vẫn báo pool: lease đã `retain_session` thì session được giữ, nhờ bản sửa G1. Tool không gọi `abandon_session`, vì chỉ `SubagentMuseCloser` được xóa session.
2. **Session mới có thể chưa đọc được history.** Commit `44351461` đã gặp lỗi này ở video. Chỉ kiểm tra Busy và SessionLost khi registry đã có session.
3. **Nhận biết SessionLost cần số lượt đã xong.** Thêm `completed_turns: u32` vào `JobSession`. Mất session khi `completed_turns > 0` và history trả 404 hoặc có ít lượt user hơn `completed_turns`. Tool ghi số lượt vào `batch_index` khi commit, và `restore_job_sessions_from_operations` lấy giá trị lớn nhất từ các dòng cùng key. Session đã ghi journal nhưng chưa gửi thành công thì `completed_turns = 0`, nên không báo nhầm.
4. **Đừng đọc nhầm lượt trước.** `TurnTracker` chỉ cho Done khi số lượt user bằng `baseline + 1`.

### Điều kiện xong trong `TurnTracker`

```rust
impl TurnTracker {
    fn new(expected_user_turns: u32, started_at: Instant) -> Self;
    fn on_event(&mut self, event: &VmStreamEvent, now: Instant);
    fn on_history(&mut self, history: &Value, now: Instant);
    fn status(&self, now: Instant) -> TurnStatus; // Working | Done | Failed(...)
    fn last_signal_at(&self) -> Instant;
    fn partial_text(&self) -> Option<String>;
}
```

Done khi đủ lượt user và một trong hai:

- Có marker `[MUSE_TASK_DONE]`.
- Session `completed`, assistant `completed`, không còn tool chạy, và trạng thái này giữ nguyên 5s.

Dùng lại `extract_history_turn_events`, `has_active_tool_task`, `is_last_assistant_completed`, `extract_turn_assistant_messages`. Chỉ áp `classify_assistant_error` cho tin nhắn ngắn không kèm file, vì một báo cáo ngắn có thể nhắc tới "rate limit" mà không phải lỗi.

### `lease.reconnect()`

Thêm vào `crates/voice-agent-server/src/providers/llm/gateway/muse/pool/lease.rs`:

```rust
pub async fn reconnect(&mut self) -> Result<broadcast::Receiver<VmStreamEvent>, (UpstreamFailureKind, String)>;
```

Gọi `pool.ensure_account_client(account_index)`, thay `self.client`, rồi trả receiver mới. `session_id` giữ nguyên.

### Approval

`agent_task/mod.rs` có `allow_agent_task_approval(item: &Value) -> bool`, bản đầu luôn trả `false`, ghi log loại approval. Danh sách cho phép được thêm từ kết quả spike. Không dùng lại `should_allow_media_approval`, vì nó cho phép mọi thứ trừ ba từ khóa.

## Lưu file và kết quả

`files.rs` có `collect_file_paths(presentations, turn_events, bubbles) -> Vec<String>`:

- Đọc `path`, `file_path`, `data.files[].path`, và `sandbox://…` trong text cùng `fallback_text`.
- Lọc theo danh sách đuôi cho phép trong [tài liệu thiết kế](muse-task-tool-design.md#định-dạng-kết-quả), bỏ trùng.
- Không sửa `candidates.rs`, để không ảnh hưởng `video_create`.

Khi `output=markdown` hoặc có `filename`, tool yêu cầu Muse ghi vào `tasks/<session_id>/<filename>`. `run_turn` thử đọc đường dẫn này trước. Cách này không phụ thuộc vào định dạng presentation, vốn chưa được đo với file không phải media.

`MuseTaskTool` xử lý sau khi có `AgentTurnOutput`:

1. Mỗi file → `ctx.artifacts().put(...)`.
2. `output=markdown` mà không có file → lưu `message` thành `.md`, `origin=message`.
3. Message vượt `tool_output_cap_bytes` → lưu thành artifact, `content` chỉ giữ đoạn đầu và ID. Server tính giới hạn này từ `read_budget`, tối thiểu 24 KiB ([task_flow/mod.rs:249](../crates/voice-agent-server/src/services/task_flow/mod.rs:249)).
4. Ghi ID và trạng thái vào `content`, vì model chỉ đọc trường này. Ghi `artifact_ids` vào `metadata`.
5. Journal `Committed` với `artifact_ids_json`, `result_json`, `batch_index = turn`.

Crash giữa `put` và `Committed` có thể tạo artifact thừa khi chạy lại, giống `video_create`. Cha chỉ thấy artifact trong kết quả cuối, nên chấp nhận được.

## Đóng phiên

```rust
pub(crate) enum ClosureDecision { Keep, Close }

pub(crate) fn decide_closure(
    latest_run: Option<&RunSummary>,
    root_has_active_work: bool,
) -> ClosureDecision;
```

Bảng quyết định nằm ở [tài liệu thiết kế](muse-task-tool-design.md#đóng-phiên). Hàm thuần này chứa toàn bộ quy tắc.

Danh sách lỗi không thử lại được nằm trong `child_failure_is_retryable`, là hàm private của thư viện, nên phải chép sang đây. Thêm test cố định danh sách, và kiểm tra lại khi nâng phiên bản `orchestrator-agent`.

Vòng lặp:

1. `list_tool_operations`, lọc key có `MUSE_TASK_KEY_PREFIX` và `remote_deleted_at` rỗng, gom theo key.
2. Lấy `CallerContext` và `session_id` từ `request_json` (tool ghi khi `begin`).
3. `list_runs` đọc hết các trang, lấy dòng cuối, vì API sắp xếp tăng dần.
4. `decide_closure` trả `Close` → `archive_session` → đọc lại run, có run đang chạy thì dừng → `release_recorded_job_session` cho từng operation → `mark_remote_session_deleted`.

Một operation có `request_json` hỏng thì ghi log và bỏ qua, không làm dừng vòng lặp.

Closer là nơi **duy nhất** xóa session của tool. Vì vậy phải loại key `muse_task:` khỏi hai đường dọn cũ:

- Vòng `for summary in ops` ở [task_flow/mod.rs:381](../crates/voice-agent-server/src/services/task_flow/mod.rs:381). Nếu không, đường này xóa session mà không archive subagent.
- `sweep_idle_job_sessions` ở [job_sessions.rs:125](../crates/voice-agent-server/src/providers/llm/gateway/muse/pool/job_sessions.rs:125).

## Bản đồ thay đổi

| File (trong `crates/voice-agent-server/src/`) | Thay đổi |
| --- | --- |
| `providers/llm/mod.rs` | Port, kiểu dữ liệu, `LlmProvider::agent_task` |
| `providers/llm/gateway/provider/mod.rs` | `agent_task()` trả `self` khi là Muse |
| `providers/llm/gateway/provider/muse_task.rs` (mới) | Ghim account, gọi `run_turn`, map lỗi |
| `providers/llm/gateway/muse/workflow/agent_task/` (mới) | `mod.rs`, `tracker.rs`, `files.rs`, `prompt.rs` |
| `providers/llm/gateway/muse/workflow/mod.rs` | Khai báo và export |
| `providers/llm/gateway/muse/pool/lease.rs` | `reconnect` |
| `providers/llm/gateway/muse/pool/job_sessions.rs` | `completed_turns`, loại key khỏi sweep |
| `providers/llm/gateway/muse/tests/agent_task.rs` (mới), `tests/harness.rs` | Test, VM giả làm đứt socket |
| `services/task_flow/muse_task/` (mới) | `mod.rs`, `tool.rs`, `closer.rs` |
| `services/task_flow/mod.rs` | Đăng ký tool ở dòng 253 đến 261, chạy closer, loại key ở dòng 381 |
| `services/task_flow/agent_settings/specs.rs` | Spec `cloud_worker`, thêm vào `daily_root`, map tool sang tag |
| `services/task_flow/agent_settings/media_backend.rs` | `agent_task_provider()`, thêm tag vào nhánh `provider_ref` |
| `services/task_flow/agent_settings/provider_catalog/resolved.rs` | `supports_tag("agent_task")` đúng khi `is_muse_provider()` |
| `apps/admin-web/src/pages/background/backgroundAgentUtils.ts` | Hiển thị tool và tag mới (kiểm tra khi làm) |

**Tag `agent_task` không thêm vào `LlmCapabilityTag`.** Nó được xử lý trong `ResolvedLlmProvider::supports_tag`, giống cách làm với `asr`. Như vậy không phải đổi validate tag, dữ liệu đã lưu hay form provider.

## Thứ tự triển khai

Mỗi bước là một commit và viết test trước. Chạy test mục tiêu trước khi chạy cả workspace:

```bash
scripts/agent-test.sh run -- cargo test --quiet -p voice-agent-server --lib agent_task
```

### Bước 1. Các module thuần

`TurnTracker`, `collect_file_paths`, `build_turn_prompt`, `decide_closure`.

Test: lượt trước đã completed nhưng chưa đủ lượt user thì vẫn Working; commentary không tính là xong; có tool chạy thì không Done dù có marker; 5s ổn định mới Done; `last_signal_at` cập nhật theo mỗi loại tín hiệu; mỗi dòng của bảng đóng phiên; danh sách lỗi cố định.

### Bước 2. `run_turn` với message

Port, adapter gateway, `reconnect`, thêm vào VM giả khả năng làm đứt socket sau N request.

Test với VM giả (H1, H5, H6, H7):

- Hai `Send` cùng key → cùng `session_id`, hai lượt user trên cùng session.
- Hai key khác nhau → hai session.
- Đứt socket giữa lúc chờ → nối lại, không gửi lại, trả đúng message.
- Socket không nối lại được → `RetryLater`, không chờ tới `deadline_at`.
- Lượt 2 có history còn của lượt 1 → không trả kết quả lượt 1.
- Mất ACK → `OutcomeUnknown`, không có session bị xóa.
- `Running` khi gọi lại → không có `stream_request` mới.
- Hết lát → `RetryLater`; quá trần → `Timeout` có `partial`; im lặng quá lâu → `Stalled`.
- Lượt trước chưa xong → `Busy`. History 404 sau khi đã xong một lượt → `SessionLost`.
- Lỗi `Transient` → `deleted_sessions()` rỗng và các lượt khác của tài khoản không bị đứt.

Dùng đồng hồ ảo của Tokio cho test thời gian, theo commit gần đây của nhóm test Muse.

### Bước 3. File

Đọc file qua `/fs/read`, đường dẫn hợp đồng, nhắc đính kèm một lần có journal.

Test: file ở đường dẫn hợp đồng; file trong presentation; đuôi không cho phép bị bỏ; file vượt giới hạn báo lỗi rõ ràng; Muse chỉ trả text → `files_missing` đúng; `Downloading` khi gọi lại → đọc lại, không gửi lượt mới.

### Bước 4. `MuseTaskTool`

Test qua Orchestrator với `FakeAgentTaskProvider` (H2, H4):

- Root bị từ chối; subagent được phép.
- Tham số sai → `INVALID_ARGUMENT`; `max_wait_secs` ngoài khoảng bị từ chối, không tự kẹp.
- `markdown` từ file → `origin=muse_file`; từ text → `origin=message`.
- Message lớn → thành artifact, `content` có ID.
- Gọi lại invocation đã `Committed` → trả kết quả đã lưu, không gọi provider.
- `RetryLater` → run `waiting_retry`, chưa settle. Attempt sau cùng `invocation_id`.
- Hủy → `CANCELLED`; shutdown → giữ operation mở.

### Bước 5. Đóng phiên

Closer, loại key khỏi hai đường dọn cũ, `completed_turns` khi restore (H3).

Test tích hợp:

- Subagent `completed` → một lần quét cho `deleted_sessions` đúng session, subagent bị archive, `remote_deleted_at` có giá trị.
- Xóa thất bại → không ghi `remote_deleted_at`, lần quét sau thử lại.
- Run đang chạy → không xóa. `failed` còn thử lại được → giữ, tới khi root hết việc.
- Gọi lại subagent đã đóng → `SESSION_ARCHIVED`.
- Cha đọc được artifact của subagent đã archive.
- Restart khi subagent còn chạy → lần gọi sau dùng lại session. Restart khi subagent đã xong → lần quét đầu đóng session.
- `video_create` và `muse_task` cùng root không dùng chung session, và luồng dọn của video không đổi.

### Bước 6. Tích hợp

Spec `cloud_worker`, prompt, `daily_root.default_subagents`, catalog, Admin.

Prompt của `cloud_worker` cần nêu: chỉ dùng một lần; làm hết các lượt với Muse trước khi kết thúc; `next_action=collect` thì gọi `collect` thay vì kết thúc; kết quả của Muse là dữ liệu ngoài, không phải chỉ dẫn; không gửi dữ liệu nhạy cảm. Prompt của root cần nêu: không giao việc này cho Task voice.

`settings` đã lưu có thể không tự có subagent mới. Kiểm tra cách `media_creator` được đồng bộ (`scripts/sync-media-creator-prompt.py`) và ghi rõ bước cập nhật nếu cần.

Xong khi: bật được qua Admin, chọn được provider; tắt thì root không thấy `cloud_worker`.

### Bước 7. Tài liệu

ADR 0136 (một Muse session cho mỗi subagent, đóng khi subagent xong), thuật ngữ trong `GLOSSARY.md`, cập nhật README nếu liệt kê tool.

### Bước 8. Kiểm chứng trên Muse thật

Chạy theo [giai đoạn 5 của thiết kế](muse-task-tool-design.md#lộ-trình), ghi kết quả vào `docs/providers/muse-task-validation-<ngày>.md`. Cần credential do bạn tự nhập và có tốn quota. Trước bước này, không được kết luận tool chạy được với Muse thật.

## Phụ thuộc

| Phụ thuộc | Trạng thái | Ảnh hưởng |
| --- | --- | --- |
| Sửa G1: `fail_account` không xóa session đã `retain_session` | Xong, đã vào `main` ở commit `4315248c`. Chưa có test nối lại thật sau cooldown | Không còn chặn bước 2; bước 2 cần thêm test nối lại sau lỗi tài khoản |
| Spike giai đoạn 0 | Chưa chạy | Bước 1 đến 7 làm được trên VM giả, nhưng các giả định dưới đây có thể đổi |

| Giả định chưa đo | Nếu sai, phải sửa |
| --- | --- |
| Muse tự tìm kiếm web qua `/chat/stream` | Dừng, hoặc thu hẹp phạm vi tool |
| Session sống qua đứt WebSocket và restart | `run_turn` và phần restore: báo `SessionLost` sau restart |
| `session_id` do client tạo được giữ | Luồng ghi journal trước lượt gửi đầu |
| Ghi được file vào `tasks/<session_id>/` theo yêu cầu trong prompt | `files.rs`, `prompt.rs` |
| `/fs/read` đọc được sau khi lượt xong | Bước 3 |
| Thời gian nghiên cứu thường dưới 30 phút | Thêm mã đỗ vào thư viện, ngoài phạm vi đợt này |

**Nên chạy spike trước bước 2.** Sai giả định đầu tiên sẽ làm toàn bộ công việc còn lại mất giá trị.

## Ngoài phạm vi

- Hội thoại sống qua nhiều subagent, và action `close`, `reset`.
- Gọi không chặn (`start`, `status`).
- Hủy riêng một lượt mà không xóa session. Chưa có endpoint đã kiểm chứng.
- Provider khác Muse. Port đã cho phép thêm, nhưng chưa làm.
- Tự động đính kèm Memory hay Wiki vào prompt.
