# 04: Closer, restore và đăng ký runtime

Status: ready-for-agent
Type: task
Blocked by: 01, 02, 03

Spec: [muse-task-tool](../spec.md)

SubagentMuseCloser theo đầy đủ bảng vòng đời, cleanup ACK/race/pagination, restore completed_turns, cô lập video; đăng ký tool và closer trong task_flow/mod.rs. Test qua seam đã xác nhận.

## Comments

- 2026-11-01: Phân rã từ spec theo yêu cầu implement-spec.
