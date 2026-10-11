# Tool Execution Status qua Voice WebSocket — Hướng dẫn triển khai

> **Status:** Proposed / implementation guide — đã đơn giản hóa theo Ponytail Review; **chưa triển khai code**.
> **Target:** `main` (nhánh `feat/compact-voice-live-conversation` đã merge).
> **Scope:** Rust Voice Server, WebSocket V1 và Compact Voice Assistant trong admin web.

## 1. Quyết định

Bổ sung message server-to-client `type: "tool"`, gồm ba trạng thái `started`, `completed`, `failed`, để client biết LLM **đang thực thi tool nào**. Client opt-in bằng `ClientHello.features.tool_status=true`.

**Giữ interface nhỏ:**

- Một JSON event: `session_id`, `turn_id`, `name`, `state`.
- Không có `call_id`, `source`, `reason`, `arguments`, `result` hoặc lịch sử tool trên wire.
- Phát từ **shared Tool-round Executor** trong `SessionActor`, dùng `send_turn_control` và `OutboundMessage::TurnText` hiện có.
- Trạng thái **advisory / best-effort**: queue đầy hoặc event bị drop không được làm lỗi turn, không thay đổi side effect hay ToolResult.
- UI Compact Voice chỉ có **một `activeTool` dưới waveform**, không tạo bubble mới, không thêm `lastTool` hoặc status history.

**Ngoài phạm vi:** progress streaming, approval UI, retry, thực thi song song, database history, cập nhật LLM/provider interface, thay đổi Device MCP protocol, tạo event bus hoặc outbound queue mới.

## 2. Seam hiện có trên main

| File | Vai trò trong thay đổi |
| --- | --- |
| `crates/voice-agent-server/src/session/actor/tools/round.rs` | `dispatch_next_tool` là nơi dispatch tuần tự, `record_tool_call` là nơi ghi ToolResult chung; thêm emission tại đây. |
| `crates/voice-agent-server/src/session/actor/tools/catalog.rs` | `resolve_tool` xác nhận tên tool thuộc catalog đã admit. |
| `crates/voice-agent-server/src/session/actor/tools/external.rs` | External MCP hoàn tất async và gọi `record_tool_call`. |
| `crates/voice-agent-server/src/session/actor/mcp.rs` | Device MCP result/timeout đi qua `complete_tool_call`. |
| `crates/voice-agent-server/src/session/actor/tools/builtin_actions.rs` | Builtin có success/failure đồng bộ và async; giữ các đường hoàn tất hiện hành. |
| `crates/voice-agent-server/src/protocol/client.rs` | `ClientFeatures` đã có serde default; thêm cờ opt-in mà không đổi V1. |
| `crates/voice-agent-server/src/app/websocket.rs` | Gắn feature vào actor; writer hiện đã gate `TurnText` theo generation. |
| `apps/admin-web/src/lib/assistant/protocol.ts` | Thêm type/parser cho `tool` và bật opt-in từ web. |
| `apps/admin-web/src/stores/assistant.ts` và `components/assistant/AssistantWidget.vue` | Trạng thái tạm thời `activeTool` và status line bên dưới waveform. |

**Flow giữ nguyên:** LLM worker cung cấp `LlmRuntimeEvent::ToolCall`; `session/actor/delivery.rs` gom các call, chỉ `start_tool_batch` sau `Finished`. ToolResult tiếp tục đi vào history/LLM continuation như cũ, theo ADR `docs/adr/0060-sequential-shared-tool-round-executor.md`.

## 3. WebSocket contract V1

### 3.1. Feature negotiation

ClientHello thêm một field trong `features`:

```json
{
  "type": "hello",
  "version": 1,
  "transport": "websocket",
  "audio_params": {
    "format": "opus",
    "sample_rate": 16000,
    "channels": 1,
    "frame_duration": 60
  },
  "features": {
    "tool_status": true,
    "turn_status": true,
    "mcp": false
  }
}
```

`tool_status` mặc định `false`. Server cũ bỏ qua feature chưa biết; client cũ không nhận event mới. Không đổi protocol version, handshake hoặc codec.

Feature này có thể bật trên Physical Device hoặc Virtual Device, độc lập với `turn_status` (hiện chỉ bật cho Web Assistant) và `mcp`. Floating Chat giữ `mcp:false`, nhưng Agent vẫn có thể dùng External MCP phía server.

### 3.2. Một event, ba trạng thái

```json
{
  "type": "tool",
  "session_id": "session-123",
  "turn_id": 12,
  "name": "external.home_assistant.light_turn_on",
  "state": "started"
}
```

`state` chỉ nhận `started`, `completed` hoặc `failed`. Các field còn lại có mặt ở **cả ba trạng thái**.

| Field | Yêu cầu |
| --- | --- |
| `type` | Literal `tool`. |
| `session_id` | Chuỗi session hiện tại, theo validation hiện có. |
| `turn_id` | Số nguyên dương, JS-safe integer; để client bỏ event của turn không còn active. |
| `name` | Tên LLM-visible **đã resolve** trong admitted tool catalog, không phải raw arguments; không rỗng, giới hạn 128 ký tự. |
| `state` | `started` / `completed` / `failed`. |

**Không cần `call_id`:** executor đã bảo đảm chỉ một tool in-flight; WebSocket giữ thứ tự các control messages đã enqueue. Trong một turn, `started` mới thay thế active tool cũ, terminal event xóa active tool; `turn_id` xác định turn. Đây là contract UI trạng thái hiện tại, **không** phải public execution-tracing contract. Nếu sau này có parallel tools hoặc per-call history thì xem xét correlation ID khi có nhu cầu thực tế.

**Không cần `source`:** `ToolTarget::{Builtin,DeviceMcp,ExternalMcp}` vẫn được dùng nội bộ để dispatch, nhưng Web chỉ hiển thị tên tool. **Không cần `reason`:** `failed` đủ cho status UI; lỗi chi tiết giữ trong ToolResult/log với policy hiện tại.

Chỉ gửi status cho tool resolve thành công và có tên trong giới hạn. `unknown_tool` hoặc tên không đủ điều kiện: giữ ToolResult error cũ, **bỏ status**, không làm fail turn. JSON serialization và Vue text interpolation phải escape đúng; không đưa raw HTML vào UI.

### 3.3. Ý nghĩa thời điểm

- `started`: ngay trước khi executor dispatch tool **sau** khi các gate và budget pass, catalog resolve thành công. Có thể tiếp theo là lỗi validation arguments đồng bộ.
- `completed`: tool-level outcome thành công đã được record; không có nghĩa audio đã phát xong.
- `failed`: tool-level outcome lỗi đã được record; không có nghĩa toàn bộ conversational turn thất bại.
- **Không** phát `started` chỉ vì LLM stream ra ToolCall; một batch có thể bị reject trước execution.
- **Không** tổng hợp `failed` khi turn bị abort, shutdown hoặc hết execution budget khi call đang chạy mà không có ToolResult.
- Event mang tính best-effort, nên **không bảo đảm** mọi `started` có terminal trên wire.

## 4. Rust implementation — không thêm module mới

### 4.1. Opt-in và outbound

1. `protocol/client.rs`: thêm `#[serde(default)] pub tool_status: bool` vào `ClientFeatures`.
2. `session/actor/mod.rs`, `construct.rs`: thêm `tool_status: bool` mặc định false, setter `with_tool_status(bool)` tương tự pattern `with_turn_status`.
3. `app/websocket.rs`: sau ClientHello, chuyển `hello.features.tool_status` vào actor, **không** phụ thuộc `assistant_connection`, `features.mcp` hoặc `features.turn_status`.
4. Phát event bằng `send_turn_control(payload.to_string())` trong `session/actor/ingress.rs`, qua `OutboundMessage::TurnText { generation, turn_id, text }`; writer đã có `GenerationGate`. **Không** dùng `send_control` / `OutboundMessage::Text`.
5. Nếu `try_send` trả Full/Closed: bỏ event và log bounded (nếu cần); tuyệt đối không `fail_closed` hoặc `fail_speech_delivery` vì observational event.

### 4.2. Private seam trong round.rs

Đặt helper trong `session/actor/tools/round.rs`; không tạo file `tool_status.rs`, enum nguồn, outcome wrapper, worker hay pubsub.

Pseudo-Rust (gợi ý shape, không phải patch compile-ready):

```rust
enum ToolStatusState { Started, Completed, Failed }

fn emit_tool_status(&self, name: &str, state: ToolStatusState) {
    // Return unless opt-in + active turn + admitted, bounded name.
    // Serialize {type:"tool", session_id, turn_id, name, state}.
    // send_turn_control; a dropped observational event is not a turn failure.
}
```

Chỉ một enum mới phục vụ wire state. Thành/bại của tool có thể truyền bằng `bool succeeded` vào `record_tool_call`, không tạo `ToolExecutionOutcome` hoặc `ToolStatusSource` riêng.

**Điểm `started`:** trong `dispatch_next_tool`, giữ nguyên thứ tự:
1. Kiểm tra batch generation/admission và lấy call.
2. Kiểm tra execution budget.
3. `resolve_tool(&call.name)`. Nếu `None`, xử lý `unknown_tool` theo flow cũ và không phát status.
4. Nếu resolve thành công, gọi `emit_tool_status(name, Started)` **một lần** trước `match ToolTarget` hiện có.

Không emit trước `start_tool_batch`/`LlmRuntimeEvent::Finished`, cũng không sửa cách executor gọi Builtin, Device MCP, External MCP.

**Điểm terminal:** giữ `record_tool_call` làm seam duy nhất ghi kết quả và phát status sau khi append `ChatMessage::ToolResult`:
- Đổi private interface thành `record_tool_call(call, content, succeeded: bool)` (hoặc cách tương đương với ít thay đổi nhất).
- `complete_tool_call` truyền success dựa trên `Result` và `raw.isError` trước khi normalize. Nhánh Xiaozhi direct response/early return cũng gọi recorder với success.
- `complete_builtin_tool_call` truyền true khi completion success; lỗi Builtin đi qua `complete_tool_call(Err(...))`.
- `complete_external_tool_call` truyền `outcome.map(|value| !value.is_error).unwrap_or(false)`, **không** parse content JSON để phân loại.
- Recorder ghi kết quả như cũ rồi emit `Completed` hoặc `Failed` **chỉ khi batch vẫn active và tool resolve được**. Nếu batch không còn, không emit. Không ghi thêm thông tin vào global history.

**Lý do không parse ToolResult:** Device MCP thông thường là `{"ok":false,"code":"..."}`, còn External MCP transport error có thể là `{"error":"external_tool_timeout"}`. `Result`/`isError`/`is_error` đã có ngay tại completion seam; dùng trực tiếp, không thêm quy tắc JSON chung.

Không thay đổi `providers/llm`, `workers/llm`, `tools/external_mcp` hoặc semantics của `McpResultDelivery` (LlmThenTts, DirectTts, Silent).

## 5. Compact Voice web implementation

### 5.1. Protocol và client

`apps/admin-web/src/lib/assistant/protocol.ts`:

```typescript
export type ToolStatusEvent = {
  type: 'tool'
  session_id: string
  turn_id: number
  name: string
  state: 'started' | 'completed' | 'failed'
}
```

Thêm vào `ServerEvent` union và `parseServerEvent`; validate type, state, `session_id`, `turn_id` safe integer, `name` non-empty / tối đa 128 ký tự. Không thêm `ToolSource`, `call_id` regex hoặc `reason` parsing. Bật `clientHello.features.tool_status=true`; giữ `turn_status:true` và `mcp:false`.

`voiceClient.ts` forward event qua callback sẵn có; **không** thay đổi state machine chỉ vì `tool:completed`. Server cũ không emit thì UI tiếp tục hiển thị `processingHint` như hiện nay.

### 5.2. Store và giao diện

`apps/admin-web/src/stores/assistant.ts`:
- Thêm **một** ref `activeTool: { name: string; turn_id: number } | null`, không lưu history và không thêm transcript message.
- `tool:started` khi đang ở active processing turn => thay `activeTool`. Terminal `completed/failed` của turn đó => clear, kể cả tên tool đã lặp trong cùng turn (executor xử lý tuần tự).
- Nếu thiếu `started` hoặc status bị drop, không coi là protocol error; terminal không có active tool thì bỏ qua.
- Reset khi turn terminal (`completed/failed/cancelled/rejected`), local abort, disconnect/reconnect, auth change hoặc bắt đầu turn mới. Bỏ status của turn đã kết thúc; dùng `turn_id` từ `turn_status` khi khả dụng.
- Không để dropped terminal khiến UI mắc kẹt sau turn kết thúc. Client vật lý tự reset transient status theo session/turn lifecycle của mình nếu không sử dụng `turn_status`.

`apps/admin-web/src/components/assistant/AssistantWidget.vue`:
- Khi processing và `activeTool`: hiện một dòng **Đang dùng {name}…** dưới waveform (icon nhỏ nếu đã có sẵn).
- Khi không có active tool: dùng `processingHint` hiện hành. **Không** thêm banner completed/failed, `lastTool`, timeline hoặc dòng tool trong Live Conversation.
- Giữ collapse ngoài panel không ngắt WS/TTS, riêng microphone capture khi ẩn vẫn interrupt như UI `main`; không sửa button đóng.
- Dùng i18n qua cơ chế `useI18n` hiện có và role/status accessible; Vue interpolation, không `v-html`.

## 6. Cancellation, ordering và failure

| Tình huống | Cách xử lý |
| --- | --- |
| Batch reject vì quota/admission/budget trước dispatch | Không có tool status; turn terminal theo flow cũ. |
| Tool sync validation fail | `started` rồi `failed` nếu đã qua dispatch gates. |
| Device MCP `isError` hoặc timeout | `failed` sau khi result được record; không lộ raw lỗi. |
| External MCP `Err` hoặc `is_error` | `failed`, dù transport response được nhận; không parse ToolResult JSON. |
| Một turn có nhiều tool hoặc nhiều rounds | `started → terminal → started → terminal` theo thứ tự executor; client chỉ giữ active mới nhất. |
| DirectTts / Silent | Vẫn phát terminal sau ToolResult, trước hand-off delivery. |
| Abort/barge-in/late MCP completion | Không record/emit stale result; generation gate drop outbound stale; UI clear khi turn kết thúc. |
| Control queue đầy | Bỏ status, tool và turn tiếp tục như hiện tại. |
| Client không opt-in | Zero `tool` event; không ảnh hưởng firmware/legacy client. |

Sự kiện tool không thay `turn_status`, không cấp thêm quyền truy cập tool, không chứa arguments, result, credentials, URL, response body hoặc error diagnostics. Status chỉ có tên đã admit và giới hạn độ dài. Trong hàng đợi control, giữ FIFO của các event được enqueue; khi message bị drop, client không được phụ thuộc vào cặp started/terminal tuyệt đối.

## 7. Triển khai và kiểm thử

**Thứ tự:**

1. Rust feature flag + emitter ở `round.rs`, bắt đầu sau `resolve_tool` và terminal tại `record_tool_call`.
2. Cập nhật nơi gọi recorder với boolean success; không thay đổi ToolResult hoặc executor scheduling.
3. Web parser/hello, Pinia `activeTool` và một dòng trạng thái tại Compact Voice.
4. Đồng bộ mô tả protocol trong `docs/flows/01-websocket.md`, `docs/flows/04-llm.md`, `docs/api/studio-assistant.md` sau khi code hoàn tất.

**Test matrix tối thiểu (cả success và lỗi):**

| Layer | Kiểm tra cần đạt |
| --- | --- |
| ClientHello | Default `tool_status=false`; true được nhận; unknown feature vẫn tương thích. |
| Actor | Chưa LLM `Finished` / round bị reject => không status. |
| Executor | Builtin, Device MCP, External MCP đều `started`/terminal đúng; nhiều calls vẫn tuần tự; unknown tool không status. |
| Outcome | Builtin argument error, Device `isError`/timeout, External `is_error`/`Err`; Xiaozhi direct response vẫn có terminal. |
| Transport | Opt-out không có event; `TurnText` generation gate; queue full không làm fail turn; late results không emit. |
| Web parser | Nhận ba states; từ chối sai session, turn_id, name hoặc state. |
| Web UI | Hiển thị active tool trong processing; xóa khi terminal/abort/disconnect; không thêm bubble; collapse vẫn giữ WS/TTS. |

Chạy các lệnh phù hợp với repository:

```bash
cargo test -p voice-agent-server --lib
cargo test -p voice-agent-server --test web_assistant
npm --prefix apps/admin-web run test
npm --prefix apps/admin-web run typecheck
npm --prefix apps/admin-web run build
```

Đây là **test plan**; chưa chạy kiểm thử implementation mới vì code chưa được thay đổi.

## 8. Definition of Done

- [ ] Message `tool` chỉ phát khi `features.tool_status=true`, tương thích WebSocket V1.
- [ ] Payload đúng bốn field nghiệp vụ/định danh `session_id`, `turn_id`, `name`, `state`; không chứa `call_id`, `source`, `reason`, arguments hoặc results.
- [ ] `started` chỉ phát tại execution seam, terminal dựa trên typed outcome và recorder hiện có.
- [ ] Builtin, Device MCP, External MCP dùng cùng contract; cancellation/backpressure không tạo regression.
- [ ] Compact Voice hiển thị `activeTool` dưới waveform, giữ nguyên transcript và Voice UX.
- [ ] Rust/web tests cần thiết pass và tài liệu WebSocket/LLM/Web Assistant được đồng bộ.

---

**Nguyên tắc cho coding agent:** sửa ít nhất tại các seam hiện có. Không thêm event bus, adapter, database state, correlation ID, status history hay abstraction cho trường hợp chưa xuất hiện. Giữ ADR 0060 và giao diện Compact Voice hiện có trên `main`.
