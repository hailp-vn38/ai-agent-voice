# 01: Port AgentTaskProvider và workflow Muse

Status: claimed
Type: task
Blocked by: none

Spec: [muse-task-tool](../spec.md)

Triển khai port theo interface trong docs/muse-task-tool-implementation.md; tracker/file/prompt, adapter, reconnect, semaphore 2/account, durable reconciliation và tải file. VM fake tests cho H1/H5/H6/H7. Sở hữu providers/llm và test Muse.

## Comments

- 2026-11-01: Phân rã từ spec theo yêu cầu implement-spec.

## Answer (source gate đạt; chờ integration check)

- Port `AgentTaskProvider` có adapter Muse, gate 2/account, khóa theo key, pin account qua registry/journal; collect không POST, Submitting đối soát baseline và replay đúng pending items; Running/Downloading không POST, lỗi journal/mất ACK giữ OutcomeUnknown.
- Workflow có tracker đúng user turn, commentary/tool-running/stability, signal clock, lát chờ/trần/stalled kèm partial, reconnect cùng account/session với 300s và backoff, giữ socket/session qua lỗi transient.
- File: ưu tiên contract, trích event/history/text, allowlist và bỏ trùng, đọc tối đa128MiB, persist Downloading paths; files thiếu có đúng một reminder checkpoint trước POST và phục hồi không gửi lại.
- Registry restore lấy completed_turns lớn nhất; completed session history404 hoặc mất turn trả SessionLost; fresh session không kiểm history trước POST. Idle sweep loại prefix muse_task:.
- VM fake coverage tại `tests/agent_task.rs`, pure tracker/path tests tách file riêng. Đã chạy TDD RED→GREEN cho contract Markdown, reminder, queued-approval race, Downloading persisted paths, report nhắc rate-limit, collect files không POST, journal hỏng và HTTP500 mơ hồ. Final targeted `agent_task`:31 PASS/0 fail (11.07s thực thi); fmt/diff check PASS. Muse regression trước hai hardening cuối:80 PASS/2 ignored. Chờ merge integration và cargo check cuối.
- G0 thật: wrapper test `muse_task_live_stage0_probe` PASS98s,2 sends cùng session, baseline0/1, turns1/2, files6174/7153bytes có SHA256, history200, delete trả thành công. Bằng chứng ngoài repo `/tmp/muse-task-stage0-evidence/report.json`; source URLs đúng xuất hiện trong báo cáo nhưng cơ chế tìm kiếm web chưa có event trực tiếp. Kill/restart khi đang chạy, loại approval thật, tác dụng delete lên filesystem/quota: NOT_RUN; không nâng thành gate PASS.
