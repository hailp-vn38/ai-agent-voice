# ADR 0132 — Single Watchdog with Three-Limit Request Timeout Model

Status: Accepted, 2026-10-31.
Extends: [ADR 0127](0127-orchestrator-flow-task-and-wiki-runtime.md), [ADR 0130](0130-internal-llm-gateway-oauth-account-pools.md).

Lượt gọi mô hình ngôn ngữ trong `orchestrator-agent` bỏ deadline cố định một mức (`request_timeout`) và chuyển sang một `Watchdog` duy nhất thực thi **Request Limits** gồm ba ngưỡng: `first_byte` (thời gian chờ tín hiệu đầu tiên), `idle` (khoảng lặng tối đa giữa hai tín hiệu hoạt động hoặc `<think>`/SSE/poll đang làm việc) và `max_total` (trần thời gian tuyệt đối của cả lượt gọi). Tầng HTTP (`reqwest`) chỉ giữ `connect_timeout` và `read_timeout = max(first_byte, idle) + 30s` làm lưới an toàn mạng thay vì áp đặt `.timeout()` tổng cắt ngang luồng đang truyền dữ liệu, còn Voice Session tiếp tục bỏ qua tín hiệu `Activity` để giữ deadline phản hồi tức thời. Mỗi lần gọi đều ghi nhận số đo (`first_byte_ms`, `last_activity_ms`, `chars_received`, `outcome`) vào bảng `llm_attempts` để phân biệt chính xác lỗi treo (`PROVIDER_STALLED`) với chạm trần tổng (`REQUEST_LIMIT_EXCEEDED`).

## Adapter coverage — 2026-10-31

Timeout profile được dùng chung cho gateway, các adapter OpenAI trực tiếp và cấu hình speech catalog để tránh mỗi adapter diễn giải `timeout_ms` khác nhau. Voice Session vẫn giữ deadline phản hồi của tầng worker. Deadline nội bộ Muse chat luôn bằng `max_total + 30s`, kể cả cấu hình cũ có `timeout_ms < 60000`, để watchdog của orchestrator báo lỗi trước lưới an toàn của adapter.
