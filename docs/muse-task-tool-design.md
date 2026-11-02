# Thiết kế tool `muse_task`: subagent giao việc cho Muse trên cloud

Trạng thái: đề xuất, chưa triển khai. Cập nhật 2026-11-01.
Liên quan: [ADR 0130](adr/0130-internal-llm-gateway-oauth-account-pools.md), [ADR 0132](adr/0132-single-watchdog-three-limit-timeout-model.md), [ADR 0133](adr/0133-durable-waiting-retry-and-job-budgets.md), [ADR 0134](adr/0134-resumable-tool-operations-and-job-muse-sessions.md).

Đường dẫn `orchestrator_agent_rs/…` và `orchestrator_task_rs/…` trỏ tới mã nguồn thư viện trong `../libs/`, bản 0.6.0 và 0.3.0.

## Kết luận

Xây một Host Tool `Resumable` tên `muse_task`, chỉ dùng trong subagent. **Một phiên subagent ứng với đúng một Muse session.** Mọi lần gọi tool trong subagent đó dùng chung session. Khi subagent hoàn thành, hệ thống xóa Muse session và lưu trữ (archive) phiên subagent.

Ba quyết định chính:

1. **Vòng đời bám theo subagent.** Khóa là `scope.session_id` của subagent, không phải root. Root gọi tool thì bị từ chối bằng `MUSE_TASK_SUBAGENT_ONLY`.
2. **Timeout không phá hủy việc đang chạy.** Trong lúc subagent còn sống, hết hạn chỉ dừng việc chờ. Session còn nguyên, và lần gọi sau lấy kết quả chứ không gửi lại yêu cầu.
3. **Không thêm bảng mới và không sửa thư viện.** Dùng lại `tool_operations`, registry Job Session và các API công khai `list_runs`, `archive_session`.

**Điều kiện chặn:** chưa ai đo Muse thật. Cả chín câu hỏi spike độ bền vẫn `NOT_RUN` ([kết quả kiểm chứng](providers/agent-job-durability-validation-2026-10-31.md)), và code chưa chứng minh Muse tự tìm kiếm web được, chỉ thấy có sự kiện `browser_task`. Phải chạy giai đoạn 0 trước khi cam kết các phần phụ thuộc.

## Hệ quả của việc đóng khi hoàn thành

Thư viện cho phép cha gọi lại subagent bằng `session_id` để tiếp tục hội thoại. Sau khi đóng, lời gọi đó nhận `SESSION_ARCHIVED`. Nghĩa là:

- Muốn hỏi thêm **sau khi** subagent trả kết quả, cha phải tạo subagent mới. Muse session mới không còn ngữ cảnh cũ, nên cha phải đưa lại kết quả trước đó vào prompt.
- Muốn trao đổi nhiều lượt với Muse, subagent phải làm hết các lượt đó trước khi kết thúc.
- Follow-up của Task cũng chạy theo cách này, vì root tạo subagent mới.

Mô tả của `cloud_worker` phải nói rõ subagent này chỉ dùng một lần.

## Hiện trạng

### Dùng lại được

| Thành phần | Vị trí | Dùng cho |
| --- | --- | --- |
| Gửi lượt, đọc history, đọc file trên VM | `/chat/stream`, `/chat/history`, `/fs/read` ([ops.rs](../crates/voice-agent-server/src/providers/llm/gateway/muse/client/ops.rs)) | Lõi của tool |
| Kết nối theo tài khoản, ping 25s, cấp lại WSS | [connect.rs](../crates/voice-agent-server/src/providers/llm/gateway/muse/pool/connect.rs) | Nối lại kết nối |
| Lease trên session có sẵn | `acquire_existing_session` ([pool/mod.rs:165](../crates/voice-agent-server/src/providers/llm/gateway/muse/pool/mod.rs:165)) | Gọi lại đúng session |
| Registry khóa theo job, khôi phục sau restart, xóa có xác nhận | [job_sessions.rs](../crates/voice-agent-server/src/providers/llm/gateway/muse/pool/job_sessions.rs) | Giữ và đóng session |
| Journal `tool_operations`, đối soát `submitting` | `orchestrator_task_rs/src/tools/media_video.rs:531`, `count_remote_user_turns` | Chống gửi trùng khi restart |
| Nhận biết lượt xong | `has_active_tool_task`, `is_last_assistant_completed` ([workflow/mod.rs:161](../crates/voice-agent-server/src/providers/llm/gateway/muse/workflow/mod.rs:161)) | Điều kiện hoàn tất |
| Trạng thái run và archive | `Orchestrator::list_runs`, `archive_session` (`orchestrator_agent_rs/src/orchestrator/`) | Phát hiện subagent xong và đóng |
| VM giả cho test | [tests/harness.rs](../crates/voice-agent-server/src/providers/llm/gateway/muse/tests/harness.rs) | Test không tốn quota |

### Khoảng trống phải xử lý

| # | Vấn đề | Hậu quả với yêu cầu này |
| --- | --- | --- |
| G1 | **Đã sửa** ở commit `4315248c` trên `main`: `fail_account` không còn xóa session khi lease đã `retain_session` ([lease.rs](../crates/voice-agent-server/src/providers/llm/gateway/muse/pool/lease.rs)) | Lỗi tài khoản giữa chừng không còn xóa session của subagent |
| G2 | Lỗi `Transient` đóng WebSocket dùng chung của tài khoản ([account.rs:112](../crates/voice-agent-server/src/providers/llm/gateway/muse/pool/account.rs:112)) | Một session lỗi làm đứt mọi lượt đang chạy trên tài khoản đó |
| G3 | Reaper hiện chỉ theo dõi Task gốc ([task_flow/mod.rs:341](../crates/voice-agent-server/src/services/task_flow/mod.rs:341)) | Không biết khi nào một subagent xong, phải thêm bước đóng |
| G4 | `begin_operation` gán `continuation_key` bằng root ([tool_operation.rs:105](../../libs/orchestrator_agent_rs/src/store/tool_operation.rs)) | Phải ghi đè, nếu không sẽ đụng session của `video_create` |
| G5 | Chỉ nhận đuôi ảnh và video ([candidates.rs:259](../crates/voice-agent-server/src/providers/llm/gateway/muse/workflow/media/candidates.rs:259)) | Không lấy được `.md`, `.pdf`, `.csv` |
| G6 | Đường chat từ chối mọi approval ([chat/mod.rs:429](../crates/voice-agent-server/src/providers/llm/gateway/muse/workflow/chat/mod.rs:429)); đường media từ chối chuỗi có `bash`, `curl`, `wget` | Việc nghiên cứu cần công cụ có thể bị chặn hoặc treo |
| G7 | Trong session dùng lại, history có thể còn là lượt trước khi lượt mới chưa ghi xong | Trả nhầm kết quả cũ. Phải so số lượt user với `user_turns_before + 1` |

## Vì sao đang bị timeout

Có năm tầng giới hạn độc lập. Một việc nghiên cứu chạm tầng nào trước thì dừng ở đó.

| Tầng | Giá trị | Khi chạm |
| --- | --- | --- |
| Tool mặc định | 300s (`orchestrator_agent_rs/src/config.rs:144`); `video_create` riêng 1800s | Còn operation mở: `TOOL_OUTCOME_UNKNOWN`, run `interrupted`, session chặn input mới tới khi đối soát |
| Muse chat | `max_total + 30s`, profile thường 600s ([muse_ops.rs:110](../crates/voice-agent-server/src/providers/llm/gateway/provider/muse_ops.rs:110)) | Bỏ dở lượt, lease bị drop và session bị xóa |
| Video | 900s + 800s mỗi lô thêm | Operation `expired` |
| Job Budget | Voice 5 phút; Chat/API 2 giờ chạy, 24 giờ tổng (`orchestrator_task_rs/src/runtime/dispatch_core.rs:18`) | Ép dừng. Chờ chặn trong tool có tính vào ngân sách |
| Retry bền | 10 lần, chờ 60, 300, 900, rồi 1800s (`orchestrator_agent_rs/src/runtime/terminal.rs:11`) | `TOOL_RETRY_LATER` tiêu lượt này, nên không hợp để thăm dò đều đặn |

Ngoài giá trị cấu hình, đọc code thấy ba lỗi làm mất thời gian mà không báo gì:

- **Đứt WebSocket là chờ tới hết hạn.** Sau khi socket đóng, `send_request` trả lỗi ngay, nhưng vòng lặp bỏ qua lỗi bằng `if let Ok` và không nối lại ([chat/mod.rs:287](../crates/voice-agent-server/src/providers/llm/gateway/muse/workflow/chat/mod.rs:287), [media/mod.rs:782](../crates/voice-agent-server/src/providers/llm/gateway/muse/workflow/media/mod.rs:782)). Đây có thể là nguyên nhân của lỗi video 900s, nhưng mới là giả thuyết: câu hỏi 6 của spike chưa được đo.
- **Chờ slot không có hạn.** Tài khoản đủ 8 thao tác thì `acquire_internal` chờ `notified()` vô hạn ([pool/mod.rs:319](../crates/voice-agent-server/src/providers/llm/gateway/muse/pool/mod.rs:319)).
- **Thăm dò dày.** 400ms mỗi lần, tức khoảng 4.500 request trong 30 phút, mỗi lần đọc history của session.

## Phương án cho timeout

| Phương án | Ưu | Nhược | Kết luận |
| --- | --- | --- | --- |
| A. Chặn một lần với timeout thật dài | Đơn giản | Giữ slot và tiêu Job Budget suốt thời gian chờ | Chưa đủ |
| B. Ba tool `start`, `status`, `result` | Subagent làm được việc khác trong lúc chờ | Mỗi lần hỏi tốn một step LLM, subagent dễ bỏ cuộc rồi kết thúc, làm Muse session bị đóng theo | Không chọn |
| **C. Chặn theo lát, nối lại bền** | Subagent chỉ thấy một lời gọi; sống qua restart; có trần rõ ràng | Cần viết vòng chờ mới | **Chọn** |

Cách C hoạt động như sau:

- `HostTool::timeout()` là 1800s, giống `video_create`. Vòng chờ nội bộ dừng ở **1700s**, để tool luôn tự kết thúc trước khi runtime cắt.
- **Trần tuyệt đối** của operation mặc định 3600s, chỉnh được bằng `max_wait_secs` trong khoảng 60 đến 7200s, và được lưu trong `deadline_at`.
- **Phát hiện treo:** 600s không có tín hiệu nào từ Muse thì coi là `MUSE_TASK_STALLED`. Tín hiệu gồm delta, `agent.status`, `task.*` hoặc history thay đổi.
- Hết lát mà Muse vẫn chạy, hoặc gặp lỗi tạm thời: trả `TOOL_RETRY_LATER`. Subagent sang `waiting_retry`, nhả slot, và attempt sau nối lại cùng `invocation_id` mà không gửi lại prompt. Subagent chưa hoàn thành nên Muse session vẫn được giữ.
- Đứt socket: nối lại ngay trong vòng chờ với backoff 1, 2, 5, 10, 30s, rồi đọc history để bắt kịp. Sau 5 phút vẫn không nối được thì `TOOL_RETRY_LATER`.
- Chạm trần: trả kết quả thất bại `MUSE_TASK_TIMEOUT` kèm phần text đã có và `next_action=collect`. Subagent quyết định chờ thêm bằng `collect`, hay kết thúc và báo kết quả dở dang. **Kết thúc thì session bị xóa và việc đang chạy bị bỏ.**
- Thăm dò chuyển sang dựa vào sự kiện, với nhịp giãn dần 2s, 5s rồi 15s. Chờ slot có hạn 60s.

Backoff của `waiting_retry` có thể cộng thêm tới 30 phút độ trễ sau nhiều lần nối lại. Bản đầu chấp nhận vì đa số việc được kỳ vọng xong trong lát đầu. Nếu spike cho thấy việc thường dài hơn 30 phút, thêm một mã đỗ riêng trong `orchestrator_agent_rs`, không tính vào `max_resume_attempts` và chờ cố định 60s.

Task nguồn voice có ngân sách 5 phút, và `ToolContext` không cho tool đọc ngân sách. Việc dài sẽ dừng ở `BUDGET_EXHAUSTED`, rồi session bị đóng. Vì vậy prompt của root chỉ nên giao `cloud_worker` với Task nguồn Chat hoặc API; giai đoạn 4 phải kiểm tra điều này.

## Thiết kế tool

### Giao diện

```jsonc
muse_task({
  "action": "send",             // send | collect; mặc định send
  "prompt": "Tìm hiểu X, tóm tắt, viết báo cáo",
  "output": "markdown",         // message | markdown | files; mặc định message
  "filename": "bao-cao-x.md",
  "max_wait_secs": 3600
})
```

- `send`: gửi lượt mới trên session của subagent; lần đầu thì tạo session. Nếu lượt trước chưa xong thì không gửi, trả `MUSE_TASK_BUSY` kèm `next_action=collect`.
- `collect`: lấy kết quả lượt gần nhất, không gửi gì lên Muse.

Không có `close`, vì đóng là việc của hệ thống khi subagent xong. Không có `conversation` hay `reset`, vì mỗi subagent chỉ có một session.

Kết quả. Model chỉ đọc `content`, nên ID artifact và trạng thái phải nằm trong text; `metadata` dành cho host:

```jsonc
{
  "status": "completed",        // completed | partial
  "turn": 3,
  "message": "…",               // bỏ commentary và marker giao thức
  "artifact_ids": ["…"],        // runtime kiểm tra quyền sở hữu trường này
  "files": [{ "artifact_id": "…", "filename": "bao-cao-x.md", "mime_type": "text/markdown",
              "size_bytes": 18234, "sha256": "…", "origin": "muse_file" }],
  "next_action": "none"         // none | collect
}
```

### Giữ phiên

Mỗi invocation ghi vào `tool_operations` của chính nó:

```text
continuation_key = "muse_task:<session_id của subagent>"   -- ghi đè giá trị root (G4)
account_key, remote_session_id
request_json     = { owner_id, workspace_id, session_id, pending_turn, … }
```

Lần gọi sau tra `job_entry(continuation_key)`. Sau restart, `restore_job_sessions_from_operations` nạp lại registry từ các dòng này mà không cần code mới. Tiền tố `muse_task:` giữ riêng khỏi session của `video_create` và giúp reaper nhận ra operation của tool.

Quy tắc:

- `remote_session_id` được ghi **trước** lượt gửi đầu tiên. Nếu Muse cấp `session_id` khác thì cập nhật theo.
- Gate sẵn có của `JobEntry` bảo đảm mỗi session chỉ có một lượt chạy. Session rảnh không chiếm `active_ops`.
- Luôn dùng đúng `account_key` đã ghim. Tài khoản tạm không dùng được thì `TOOL_RETRY_LATER`, không failover vì session nằm trên VM của tài khoản đó.
- Session mất trên remote giữa chừng: trả `MUSE_SESSION_LOST`, **không tự tạo mới**, để subagent báo cho cha biết Muse đã mất ngữ cảnh.
- WebSocket vẫn dùng chung theo tài khoản, không mở riêng cho mỗi subagent.

### Đóng phiên

Thêm một vòng lặp 10 giây vào `spawn_job_session_reaper` ([task_flow/mod.rs:323](../crates/voice-agent-server/src/services/task_flow/mod.rs:323)). Với mỗi operation có tiền tố `muse_task:` và chưa có `remote_deleted_at`, nó đọc attempt mới nhất của subagent qua `list_runs`, rồi xử lý theo bảng:

| Trạng thái subagent | Hành động |
| --- | --- |
| `running`, `waiting_retry`, `waiting_user` | Giữ |
| `completed`, kể cả dừng vì hết step hoặc ngân sách | **Đóng** |
| `cancelled` | **Đóng** |
| `failed` không thử lại được | **Đóng** |
| `failed` còn thử lại được | Giữ, vì cha có thể gọi tiếp |
| `interrupted` (tắt server, `TOOL_OUTCOME_UNKNOWN`) | Giữ, vì recovery có thể nối lại |
| Root không còn việc nào đang chạy | **Đóng** (lưới an toàn cho hai dòng "Giữ" ở trên) |

Thứ tự đóng tránh được việc cha gọi lại đúng lúc đang đóng:

1. `archive_session`: từ đây subagent không nhận lượt mới. Thao tác này idempotent.
2. Đọc lại attempt mới nhất. Nếu có run đang chạy thì dừng lại, lần quét sau xử lý tiếp.
3. `release_recorded_job_session`, chỉ thành công khi Muse xác nhận đã xóa.
4. `mark_remote_session_deleted`. Xóa thất bại thì không ghi, vòng lặp sau thử lại.

Vì đọc từ DB nên cơ chế này tự đúng sau restart: subagent xong lúc server tắt sẽ được đóng ở lần quét đầu tiên. Độ trễ đóng tối đa khoảng 10 giây.

Kết quả đã nằm trong artifact của job trước khi xóa session, nên xóa không mất dữ liệu. Giai đoạn 2 phải có test xác nhận cha vẫn đọc được artifact của subagent đã archive.

### Lượt làm việc

```text
muse_task(send)
  ├─ khóa session theo subagent
  ├─ journal submitting + user_turns_before
  ├─ /chat/stream ── mất ACK ─→ TOOL_OUTCOME_UNKNOWN, đối soát bằng số lượt user
  ├─ journal running
  ├─ chờ: số lượt user = before + 1, VÀ
  │       (có marker [MUSE_TASK_DONE], HOẶC session completed + assistant completed
  │        + không còn tool chạy, ổn định 5s)
  ├─ journal downloading
  ├─ /fs/read từng file → ctx.artifacts().put
  └─ journal committed + result_json → trả ToolOutcome
```

Tool bọc prompt bằng hợp đồng ngắn: làm việc trong thư mục riêng `tasks/<session_id>/`, ghi file đầu ra ở đó, nêu đường dẫn `sandbox://…`, kết thúc bằng marker. Thư mục riêng cần thiết vì nhiều session dùng chung một VM. Marker chỉ là tín hiệu nhanh; thiếu marker vẫn hoàn tất theo trạng thái session.

### Định dạng kết quả

| `output` | Muse có file | Muse chỉ trả text |
| --- | --- | --- |
| `message` | Trả text, file (nếu có) lưu thành artifact | Trả text |
| `markdown` | Lưu file `.md`, `origin=muse_file` | Lưu text thành `.md`, `origin=message`, ghi rõ trong `content` |
| `files` | Lưu các file | `status=partial`, nhắc Muse đính kèm một lần (journal chống nhắc lại), vẫn thiếu thì báo rõ |

- Tách phần trích đường dẫn khỏi `MediaKind`, thành bộ trích file tổng quát kèm danh sách cho phép: `.md .txt .pdf .csv .json .html .docx .xlsx .png .jpg .webp .mp4`.
- Mỗi file tối đa bằng `artifact_max_bytes` (128 MiB). Tên file được chuẩn hóa, không ghi ra đường dẫn cục bộ do Muse đặt.
- Text vượt `tool_output_cap_bytes` (server tính từ `read_budget`, tối thiểu 24 KiB) thì lưu thành artifact, `content` chỉ giữ đoạn đầu và ID.

### Vị trí trong code

Chi tiết nằm ở [tài liệu triển khai](muse-task-tool-implementation.md).

```text
providers/llm/mod.rs                     trait AgentTaskProvider, cạnh DurableMediaProvider
providers/llm/gateway/provider/muse_task.rs      impl cho LlmGatewayProvider
providers/llm/gateway/muse/workflow/agent_task/  vòng chờ, hoàn tất, trích file
services/task_flow/muse_task/            HostTool, bước đóng phiên
services/task_flow/mod.rs:253            register_tool
services/task_flow/agent_settings/specs.rs       subagent cloud_worker dưới daily_root
```

Đặt trong server, giống `chart_host_tools` và `memory.host_tools`. Trait giúp test bằng backend giả.

Agent: thêm subagent `cloud_worker` với tool `muse_task`. Tool map vào tag `agent_task` qua `tool_capability_tags`, nên Admin chọn provider theo cơ chế `tool_providers` đã có. Tag này do `ResolvedLlmProvider::supports_tag` xử lý, không thêm vào `LlmCapabilityTag`.

## An toàn

- **Dữ liệu đi ra ngoài.** Prompt và file gửi sang dịch vụ bên thứ ba. Tool tắt mặc định, Admin bật theo agent. Không tự đính kèm Memory hay Wiki vào prompt; thêm cảnh báo trong prompt của `cloud_worker`.
- **Kết quả không đáng tin.** Muse đọc web nên kết quả có thể chứa prompt injection. `content` được đánh dấu là dữ liệu từ nguồn ngoài, và `cloud_worker` không có tool ghi hay tool thiết bị.
- **Approval.** Mặc định từ chối, cho phép theo danh sách lấy từ loại approval quan sát được ở spike. Ghi log mọi quyết định, không ghi nội dung nhạy cảm. Từ chối phải khiến lượt báo lỗi rõ ràng, không treo tới hết hạn.
- **Tài khoản.** Provider này dựa trên giao thức không công khai, nên có thể hỏng khi Muse đổi và tài khoản có rủi ro. Giới hạn lượt chạy đồng thời của tool (mặc định 2 mỗi tài khoản), không retry khi kết quả chưa rõ.

## Lộ trình

Mỗi giai đoạn dùng `scripts/agent-test.sh`, chạy test mục tiêu trước.

**0. Spike với tài khoản thật.** Chạy [runbook có sẵn](providers/muse-durability-spike.md), thêm các câu dưới đây. Cần credential do bạn tự nhập; có tốn quota.

| Câu hỏi | Nếu không đạt |
| --- | --- |
| Muse có tự tìm kiếm web và viết báo cáo qua `/chat/stream` không? | Dừng, hoặc đổi phạm vi thành chỉ tóm tắt tài liệu đưa vào |
| Việc có tiếp tục sau khi đứt WebSocket không? (câu 1) | Không nối lại được qua restart; giữ trong một process và báo `MUSE_SESSION_LOST` |
| Thời gian thực của việc nghiên cứu là bao lâu? | Trên 30 phút: làm mã đỗ trong thư viện |
| Loại approval nào xuất hiện? | Làm đầu vào cho danh sách cho phép |
| File không phải media hiển thị ở đâu trong history? | Quyết định cách trích file, hoặc liệt kê thư mục |
| `/chat/history` có đọc từ một mốc được không? | Không: giãn nhịp thăm dò, đo kích thước |
| `/session/delete` có dừng lượt đang chạy và dọn file trên VM không? | Không: thêm bước dọn `tasks/<session_id>/`; ghi nhận quota còn bị tiêu |

**1. Lõi phiên và message.** `send`, `collect`, vòng chờ theo lát, nối lại socket. Xong khi: VM giả cho thấy hai lần gọi trong một subagent dùng một `session_id`, hai subagent dùng hai session, root bị từ chối, đứt socket tiếp tục được, không trả nhầm lượt cũ.

**2. File và Markdown.** Bộ trích file tổng quát, ghi artifact, bảng định dạng ở trên. Xong khi: kịch bản ví dụ trên VM giả cho ra artifact `.md` đúng `sha256`, kể cả khi Muse chỉ trả text, và cha vẫn đọc được artifact sau khi subagent bị archive.

**3. Độ bền và đóng phiên.** `TOOL_RETRY_LATER`, đối soát sau restart, vòng lặp đóng. Xong khi: có test cho từng dòng của bảng trạng thái; restart ở từng trạng thái không gửi trùng lượt; gọi lại subagent đã đóng nhận `SESSION_ARCHIVED`; `video_create` cùng job không dùng chung session.

**4. Tích hợp.** Tag `agent_task`, `cloud_worker`, prompt root (gồm giới hạn Task voice), Admin, ADR 0136, thêm thuật ngữ vào `GLOSSARY.md`. Xong khi: bật được qua Admin và chọn được provider.

**5. Kiểm chứng thật.** Chạy ví dụ "tìm hiểu, tóm tắt, viết báo cáo" với hai lượt `send` trong một subagent, và một lần kill server giữa chừng. Đo `session_id` không đổi trong subagent, không gửi trùng, và không còn session Muse nào sau khi subagent xong.

## Rủi ro còn lại

| Rủi ro | Giảm thiểu |
| --- | --- |
| Session không sống qua đứt socket | Spike trước; có nhánh dự phòng rõ ràng |
| Subagent kết thúc sớm khi Muse còn chạy, làm mất việc | Prompt của `cloud_worker` yêu cầu dùng `collect` trước khi kết thúc; kết quả tool nêu rõ `next_action` |
| Nhận sai "đã xong" khi Muse đang chuyển bước | Yêu cầu không còn tool chạy và ổn định 5s; có marker |
| `waiting_retry` tiêu hết 10 lần | Lát 1700s và trần 7200s giới hạn số lần cần tới; lỗi tạm thời mới tiêu lượt |
| Quét `list_tool_operations` mỗi 10 giây tốn dần khi bảng lớn | API hiện đọc toàn bảng. Nếu đo thấy chậm, thêm truy vấn lọc theo operation còn mở vào thư viện |
