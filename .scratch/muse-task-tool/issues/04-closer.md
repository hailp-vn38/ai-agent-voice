# 04: Closer, restore và đăng ký runtime

Status: resolved
Type: task
Blocked by: none

Spec: [muse-task-tool](../spec.md)

SubagentMuseCloser theo đầy đủ bảng vòng đời, cleanup ACK/race/pagination, restore completed_turns, cô lập video; đăng ký tool và closer trong task_flow/mod.rs. Test qua seam đã xác nhận.

## Comments

- 2026-11-01: Phân rã từ spec theo yêu cầu implement-spec.

- 2026-11-01: Frontier cập nhật: port đã chốt ở 373e5e99, journal scope đã thống nhất; closer có thể triển khai độc lập theo contract này. Runtime wiring và nghiệm thu tích hợp chỉ hoàn tất sau merge 01/02; không resolve sớm.

## Answer

- Implemented `SubagentMuseCloser` lifecycle decisions, paginated latest-run lookup, archive verification through public session metadata, startup sweep, and ACK-only durable deletion. Shared remote IDs across continuation keys defer cleanup; all account/remote pairs must acknowledge before journal rows are marked deleted.
- Wired `muse_task` into the runtime and excluded its continuation keys from competing cleanup paths. Restore retains completed-turn counts. Shared DELETE now treats non-2xx as failure, so retry leaves the operation unacknowledged until a successful response.
- Merged gateway workflow (`b7564312`), latest docs (`bf29151c`), and root integration `420695b5` into this branch. Closer implementation commit: `e41e135a`.
- Validation through `scripts/agent-test.sh`: fmt check PASS; `targeted muse_task_closer` PASS (10); `targeted agent_task` PASS (31); `targeted job_terminal_deletes_session` PASS (1); `targeted video_call_without_continuation_key_still_deletes_its_session` PASS (1); `check voice-agent-server` PASS. `targeted media` by substring matched no tests (0 passed), so the two named media cleanup regressions above were run directly.
