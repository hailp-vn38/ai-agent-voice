# 04: Closer, restore và đăng ký runtime

Status: ready-for-agent
Type: task
Blocked by: none

Spec: [muse-task-tool](../spec.md)

SubagentMuseCloser theo đầy đủ bảng vòng đời, cleanup ACK/race/pagination, restore completed_turns, cô lập video; đăng ký tool và closer trong task_flow/mod.rs. Test qua seam đã xác nhận.

## Comments

- 2026-11-01: Phân rã từ spec theo yêu cầu implement-spec.

- 2026-11-01: Frontier cập nhật: port đã chốt ở 373e5e99, journal scope đã thống nhất; closer có thể triển khai độc lập theo contract này. Runtime wiring và nghiệm thu tích hợp chỉ hoàn tất sau merge 01/02; không resolve sớm.
