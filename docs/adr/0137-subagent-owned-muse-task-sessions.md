# ADR 0137 — Sub-Agent sở hữu phiên Muse Task

Status: Accepted, 2026-11-02.
Extends: [ADR 0129](0129-admin-managed-subagent-catalog.md), [ADR 0130](0130-internal-llm-gateway-oauth-account-pools.md), [ADR 0133](0133-durable-waiting-retry-and-job-budgets.md), [ADR 0134](0134-resumable-tool-operations-and-job-muse-sessions.md), [ADR 0136](0136-collection-ledger-and-bounded-autoflow-batches.md).

`muse_task` là Host Tool resumable dành cho `cloud_worker` của `daily_root`, mặc định tắt. Mỗi Agent Session của con sở hữu một Job Session và Gateway Account đã ghim; Continuation Key là `muse_task:<Agent Session ID của con>`. Đây là phần mở rộng phạm vi của ADR 0134: phiên video vẫn thuộc Background Task; phiên Muse Task kết thúc theo con, không giữ tới hết root và không dùng chung với video. Chọn phạm vi này để hai con không lẫn ngữ cảnh và để kết thúc một nhiệm vụ có ranh giới dọn tài nguyên rõ ràng.

## Decisions

- `send` mở lượt mới khi lượt trước đã xong; `collect` chỉ thu hồi. Journal ghi pending turn, baseline và định danh remote trước POST, giữ nguyên qua retry/restart. Mất ACK cần đối soát; không failover sang tài khoản khác hoặc gửi việc mới khi kết quả còn mơ hồ.
- Con làm đủ các lượt rồi bàn giao message/Job Artifact trước khi kết thúc. Hỏi thêm sau khi archive tạo con mới; cha truyền lại ngữ cảnh cần thiết. Artifact đã xuất bản vẫn tồn tại để cha đọc. Timeout/stalled có thể trả partial text và yêu cầu collect nếu con còn hoạt động, nhưng không đặt lại ngân sách turn.
- `SubagentMuseCloser` là chủ sở hữu cleanup duy nhất cho tiền tố `muse_task:`. Mỗi khoảng 10 giây nó đọc attempt mới nhất qua mọi trang; giữ con đang chạy/chờ/interrupted hoặc failed còn retry được, đóng con terminal hoặc root không còn hoạt động. `completed` do step/budget vẫn đóng, không được suy thành nhiệm vụ đạt mục tiêu.
- Với API công khai `orchestrator-agent` 0.6.0, archive kiểm tra `get_session().archived` trước và xác minh lại sau lỗi; không giả định `archive_session` tự idempotent. Sau archive đọc lại run để chặn race rồi mới release. Quy tắc failed có retry phải khớp classifier của bản thư viện này và có guard phát hiện drift.
- Sửa đổi 2026-11-02: mặc định giữ phiên Muse sau khi con kết thúc để người dùng xem lại trên web Muse. Con chỉ dùng phiên khi đang chạy: closer vẫn archive con và bỏ trạng thái cục bộ, không gọi DELETE. Chỉ xóa khi một lượt của con có `delete_session_on_finish=true`, và model chỉ đặt cờ này khi người dùng yêu cầu rõ. Phiên được giữ không có TTL, nên việc Muse đang chạy dở không bị DELETE chặn lại và người dùng tự dọn trên Muse.
- Xóa remote chỉ thành công khi HTTP 2xx; chỉ sau ACK mới ghi `remote_deleted_at`. Gom theo account/session để gọi xóa, nhưng API đánh dấu chỉ nhận session ID: chỉ đánh dấu khi mọi account liên quan đã ACK; xung đột key được hoãn để không xóa nhầm phạm vi. Lỗi upstream sẽ được thử lại, nên 10 giây là nhịp phát hiện, không phải SLA xóa.
- Root cleanup và idle sweep không sở hữu phiên Muse Task; cleanup video giữ hợp đồng cũ. Không thêm bảng, migration hoặc sửa thư viện để thực hiện quyết định này.
- Admin chọn provider Muse qua `tool_providers.muse_task`; catalog dùng tag `agent_task`. Prompt nêu dữ liệu ngoài không đáng tin, không tự gửi Memory/Wiki, không giao việc dài này cho Task Voice. Approval mặc định từ chối; chỉ mở loại có bằng chứng quan sát và xác minh.

## Considered Options

Dùng phiên root chung tiết kiệm tạo session nhưng làm lẫn công việc hai con và giữ VM lâu hơn nhiệm vụ. Tạo session mỗi invocation làm mất ngữ cảnh và tăng nguy cơ gửi lại việc khi phục hồi. Thêm action close vào tool đặt trách nhiệm dọn tài nguyên vào model, không bảo đảm chạy khi con dừng vì ngân sách; closer ở server giữ trách nhiệm này.

## Evidence boundary

Quyết định và mã nguồn không chứng minh hành vi Muse thật. [Báo cáo validation](../providers/muse-task-validation-2026-11-02.md) tách stage0 khỏi gate Background Task, restart, artifact và cleanup; mục chưa đo giữ `NOT_RUN`. ACK xóa cũng không tự chứng minh hủy task, dọn file hay hoàn quota.
