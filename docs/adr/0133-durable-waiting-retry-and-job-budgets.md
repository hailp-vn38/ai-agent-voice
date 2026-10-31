# ADR 0133 — Durable Waiting Retry, Error Classification, and Job Budgets for Background Tasks

Status: Accepted, 2026-10-31.
Extends: [ADR 0023](0023-zero-provider-retries.md), [ADR 0095](0095-single-process-durable-job-recovery.md), [ADR 0127](0127-orchestrator-flow-task-and-wiki-runtime.md).

Khi một **Agent Run** của **Background Task** gặp lỗi LLM tạm thời, đứt luồng hoặc chạm giới hạn tốc độ (`RateLimited`, `Stalled`, `Transient`, `Timeout`, `Protocol`), runtime thử lại nhanh có giới hạn trước khi commit step và sau đó chuyển bền vững sang trạng thái **Waiting Retry** (`waiting_retry` kèm `next_attempt_at`, hoặc `waiting_user` khi lỗi xác thực cần Admin can thiệp), nhả slot worker cũng như không đếm Root Orchestrator đang chờ con vào hạn ngạch `max_concurrent_root_runs`. Mỗi lượt thực thi của Background Task được quản lý bởi **Job Budget** (`RunBudget`) theo dõi thời gian chạy tích lũy (không tính thời gian con nằm chờ ở `waiting_retry`), thời hạn tổng thể và khoảng dự trữ để nhắc model gói gọn từ mốc 80%, ép dừng chỉ xuất văn bản kèm kết quả từng phần (`partial_output`) ở mốc 100%, và ánh xạ mã `BUDGET_EXHAUSTED` sang trạng thái `TaskStatus::Stopped`.

## Relationship to ADR 0023 and ADR 0095

- **Ngoại lệ có phạm vi so với [ADR 0023](0023-zero-provider-retries.md)**: ADR 0023 tiếp tục cấm tự động retry bên trong tầng provider adapter và giữ nguyên nguyên tắc một lần gọi cho Voice Session; cơ chế retry mới chỉ nằm ở tầng điều phối **Agent Run** của **Background Task** trước khi step được commit.
- **Phân định với [ADR 0095](0095-single-process-durable-job-recovery.md)**: Quy tắc không tự động chạy lại các bước có tác dụng phụ không rõ kết quả (`TOOL_OUTCOME_UNKNOWN` đối với công cụ `Mutating`) của ADR 0095 vẫn giữ nguyên hiệu lực; chỉ các lượt gọi mô hình ngôn ngữ chưa commit và các công cụ khai báo `ToolEffect::Resumable` ([ADR 0134](0134-resumable-tool-operations-and-job-muse-sessions.md)) mới được tự động khôi phục và nối tiếp qua các attempt.

## Clarification — 2026-10-31

Job Budget áp dụng cho một turn và mọi attempt phục hồi của turn đó. Task Follow-up là yêu cầu mới của người dùng nên nhận ngân sách mới theo TaskSource; ngân sách không được đặt lại khi retry, restart hoặc nối lại một turn đang chờ. Cách này giữ follow-up độc lập với thời gian đã tiêu của yêu cầu trước, đồng thời vẫn chặn retry vô hạn trên cùng yêu cầu.
