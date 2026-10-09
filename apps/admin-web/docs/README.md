# Tài liệu Admin Web

Bộ tài liệu mô tả implementation trong `apps/admin-web` được rà soát ngày **2026-10-09**. Mục tiêu: giúp vận hành UI, đọc code và xác định đúng ranh giới giữa browser, Admin API và runtime backend.

| Tài liệu | Nội dung |
| --- | --- |
| [Architecture](architecture.md) | Entry points, state ownership, view models, cache, UI primitives |
| [API](api.md) | HTTP wrapper, API module map, revisions, pagination, diagnostics, credentials |
| [Workflows](workflows.md) | Hành vi các trang, enrollment, provider lifecycle, tool review, speaker recognition |
| [Development](development.md) | Cài đặt, commands, môi trường, deploy, test và xử lý lỗi |
| [Review](review.md) | Findings có tình huống cụ thể, hướng sửa nhỏ nhất và evidence |

## Cách đọc

Bắt đầu từ [README ứng dụng](../README.md), sau đó chọn workflows nếu dùng UI hoặc architecture/API nếu sửa code. Đọc review trước khi dựa vào cache counts, thao tác Device từ Agent Detail, chỉnh provider bindings qua Template form hoặc duplicate Provider.

Các trang hướng dẫn mô tả luồng UI đã có; các lỗi làm luồng chưa hoạt động đúng được dẫn sang review. Không coi nút xuất hiện trên UI là bằng chứng API hoặc runtime đã hỗ trợ đầy đủ.

## Nguồn chân lý

- [Domain context](../../../CONTEXT.md): thuật ngữ và ownership.
- [ADR 0051](../../../docs/adr/0051-separate-authenticated-admin-api.md): Admin authentication.
- [ADR 0054](../../../docs/adr/0054-admin-api-optimistic-concurrency.md): revision/CAS.
- [ADR 0070](../../../docs/adr/0070-conditional-admin-hard-delete.md): conditional hard-delete.
- [ADR 0071](../../../docs/adr/0071-versioned-provider-runtime-manager.md): desired revision và runtime version.
- [ADR 0072](../../../docs/adr/0072-sqlite-device-enrollment.md), [ADR 0074](../../../docs/adr/0074-websocket-enrollment-session.md): Device enrollment.
- [ADR 0077](../../../docs/adr/0077-speaker-v1-authority-and-calibration.md): chỉ `off`/advisory `observe` đã triển khai.
- [ADR 0082](../../../docs/adr/0082-unified-speaker-enrollment-default.md): quyết định enrollment nhiều mẫu/holdout; khác capture một mẫu đang triển khai, xem review.
- [ADR 0083](../../../docs/adr/0083-admin-managed-resource-credentials.md): credentials qua Admin API.

[ADR 0078](../../../docs/adr/0078-agent-tool-allowlist.md) đã superseded. UI hiện chỉ review External MCP tools; không suy từ nội dung cũ rằng có Device tool approval/recovery.

Các tài liệu thiết kế cũ đang được xóa trong working tree không được khôi phục. Bộ tài liệu này không giữ những mô tả cũ về mock data, SecretRef-only authentication, provider side sheet ở catalog hoặc các tính năng chỉ có trong kế hoạch.
