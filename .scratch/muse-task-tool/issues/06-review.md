# 06: Review và gate tích hợp

Status: resolved
Type: task
Blocked by: 05

Spec: [muse-task-tool](../spec.md)

Code-review Standards và Spec độc lập, sửa findings, chạy targeted rồi toàn workspace và web thích hợp qua scripts/agent-test.sh.

## Comments

- 2026-11-01: Phân rã từ spec theo yêu cầu implement-spec.

- 2026-11-02: Code review hoàn tất trên integration branch. Standards và Spec chạy độc lập; findings đã được xử lý trong `593d79ef`.

## Answer

### Standards

- Hard breach: `muse_task/tool.rs` gộp parsing, lifecycle/provider, artifact output và error mapping ở một file 565 dòng. Đã tách thành `args.rs`, `output.rs`, `failures.rs` và giữ `tool.rs` cho điều phối.
- Possible duplicated code: extension allowlist lặp giữa gateway Agent Task và Muse Task Host Tool. Đã gom vào `providers/llm/agent_task_files.rs`.
- Không có finding Standards khác; commit `121f0335` về Knowledge lexical search được xác định là thay đổi người dùng ngoài scope Muse.

### Spec

- P2: prompt root thiếu hướng dẫn tránh giao tác vụ nghiên cứu/báo cáo dài cho Voice Task có budget 5 phút. Đã thêm hướng dẫn server-owned vào prompt mặc định tại seed, settings resolution và Admin projection; prompt Admin tùy chỉnh được giữ nguyên.
- Finding concurrency cap được reviewer rút lại sau khi xác nhận `muse_task.rs` có semaphore 2 slot/account độc lập config và timeout 60 giây.
- Không có finding source khác. Các gate Muse thật chưa đủ bằng chứng được ghi `NOT_RUN` trong ticket 07, không tính là lỗi source.

### Validation

- PASS: targeted prompt/override/projection, Muse extension filtering, Agent Task path extraction, shared extension case-insensitivity.
- PASS: `scripts/agent-test.sh run -- cargo fmt --all --check`; `git diff --cached --check`.
- `scripts/agent-test.sh full` FAIL vì linker không có dung lượng trống (`errno=28`) khi tạo integration binaries; đây là lỗi hạ tầng, không phải assertion. Log: `/private/var/folders/j6/50n8d4j97m7ddxpvtwpglnjh0000gn/T/ai-agent-voice-test-logs/agent-test.full.plK227`.
- Review-fix tích hợp tại `593d79ef`. Full workspace vẫn cần chạy lại khi đủ dung lượng.
