# ADR 0023 — Không automatic retry logical provider operation trong V1

## Status
Accepted

Mỗi logical ASR, LLM, TTS hoặc MCP operation V1 chỉ thực hiện một lần sau khi đã phát dữ liệu; timeout/lỗi đi theo terminal path, không retry application/provider layer đối với các adapter khóa tĩnh hoặc MCP `tools/call` (để tránh side effect lặp). Điều này giữ latency, cost, cancellation và delivery semantics deterministic.

Ngoại lệ có giới hạn cho `LlmGatewayProvider` ([ADR-0130](0130-internal-llm-gateway-oauth-account-pools.md)): trước khi phát bất kỳ token/delta nào ra stream, nếu tài khoản OAuth/phiên máy ảo trả về lỗi xác thực (`401` / `AuthInvalid`), provider được phép gọi `try_refresh_on_unauthorized` tối đa 1 lần cho tài khoản đó (và với cấu hình `pool:<vendor>`, xoay vòng tối đa `MAX_POOL_FAILOVER_ATTEMPTS` tài khoản khỏe mạnh khi gặp lỗi `401`/`429`/transient trước khi phát dữ liệu).

## Reminder delivery attempts (proposed)

Retry bền vững ở cấp Reminder được đề xuất trong [ADR-0090](0090-reminder-playback-ack-and-bounded-attempts.md). Mỗi attempt là một logical TTS operation mới; không thay đổi lệnh cấm retry bên trong provider operation hoặc MCP call. Đây là thiết kế chưa triển khai.

## Agent Run durable retry for Background Tasks

Đối với Background Task, việc thử lại bền vững khi gặp lỗi LLM tạm thời hoặc hết hạn mức được thực hiện ở tầng điều phối Agent Run (`waiting_retry` và fast retry trước khi commit step) theo [ADR-0133](0133-durable-waiting-retry-and-job-budgets.md), trong khi Voice Session vẫn giữ nguyên nguyên tắc một lần gọi và không retry ở tầng provider.
