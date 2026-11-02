# 01: Port AgentTaskProvider và workflow Muse

Status: resolved
Type: task
Blocked by: none

Spec: [muse-task-tool](../spec.md)

Triển khai port theo interface trong docs/muse-task-tool-implementation.md; tracker/file/prompt, adapter, reconnect, semaphore 2/account, durable reconciliation và tải file. VM fake tests cho H1/H5/H6/H7. Sở hữu providers/llm và test Muse.

## Comments

- 2026-11-01: Phân rã từ spec theo yêu cầu implement-spec.

## Answer

- Port `AgentTaskProvider` có adapter Muse, gate 2/account, khóa theo key, pin account qua registry/journal; collect không POST, Submitting đối soát baseline và replay đúng pending items; Running/Downloading không POST, lỗi journal/mất ACK giữ OutcomeUnknown.
- Workflow có tracker đúng user turn, commentary/tool-running/stability, signal clock, lát chờ/trần/stalled kèm partial, reconnect cùng account/session với 300s và backoff, giữ socket/session qua lỗi transient.
- File: ưu tiên contract, trích event/history/text, allowlist và bỏ trùng, đọc tối đa128MiB, persist Downloading paths; files thiếu có đúng một reminder checkpoint trước POST và phục hồi không gửi lại.
- Registry restore lấy completed_turns lớn nhất; completed session history404 hoặc mất turn trả SessionLost; fresh session không kiểm history trước POST. Idle sweep loại prefix muse_task:.
- VM fake coverage tại `tests/agent_task.rs`, pure tracker/path tests tách file riêng. TDD RED→GREEN cho contract Markdown, reminder, queued-approval race, Downloading persisted paths, report nhắc rate-limit, collect files không POST, journal hỏng và HTTP500 mơ hồ. Sau tích hợp, targeted `agent_task`: 31 PASS/0 fail; rustfmt và `cargo check` qua wrapper PASS. Muse regression ghi nhận trước hardening cuối: 80 PASS/2 ignored.
- Tích hợp tại `36239c75`; review-fix dùng chung allowlist được tích hợp ở `593d79ef`. Live stage0 được chạy lại với account Muse-1b: hai send cùng session, files 5125/6208 bytes, history 200 và DELETE 2xx PASS; xem [validation report](../../../docs/providers/muse-task-validation-2026-11-02.md). Browser event, kill/restart khi đang chạy, approval thật, full Background Task, cleanup filesystem/quota: NOT_RUN; đây không phải nghiệm thu cuối ticket 07.
