# Kiểm chứng Muse Task — 2026-11-02

Bản chụp bằng chứng ngày 2026-11-02. `PASS` chỉ áp dụng đúng phạm vi trong bảng; `FAIL` là quan sát trái hợp đồng; `NOT_RUN` là chưa đo hoặc chưa đủ bằng chứng. Test VM giả và source checks không thay thế nghiệm thu Muse thật. ADR: [0137](../adr/0137-subagent-owned-muse-task-sessions.md); cách đo tiếp: [runbook](muse-task-runbook.md).

## Nguồn và phạm vi

Stage0 chạy `muse_task_live_stage0_probe` trên adapter thật, Gateway Account Muse-1b được người dùng cho phép tiêu quota. Probe đi qua provider port + VM Muse thật, dùng file/memory journal; không chạy đầy đủ Background Task/Orchestrator. Bản chạy lại với client chỉ công nhận HTTP 2xx lưu tại `/tmp/muse-task-stage0-reverify/report.json`; output thô và file tải về không đưa vào repo vì có thể chứa dữ liệu riêng. SHA-256 hai file được tính lại độc lập và khớp report.

## Quan sát trực tiếp stage0

| Phép đo | Trạng thái | Bằng chứng và giới hạn |
| --- | --- | --- |
| Hai `send` liên tiếp dùng một remote session | PASS | Cùng session, baseline 0/1, kết quả turn 1/2; không chứng minh hai con tách phiên |
| Message và đọc file Markdown mỗi lượt | PASS | Hai file `report.md` đọc được, kích thước/checksum bên dưới |
| Đọc history sau lượt gửi | PASS | History GET trả 200 trước cleanup |
| Thời gian hai lượt | PASS | Lượt 1: 50 giây; lượt 2: 22 giây; đây là thời gian quan sát, không phải SLA |
| Tìm kiếm/duyệt web thực sự | NOT_RUN | Có URL Tokio chính thức và 32 structured citations trong history; không có capture browser/task event nên chưa chứng minh browsing |
| Loại approval và quyết định | NOT_RUN | Không quan sát được approval trong probe |
| Xác nhận HTTP ACK xóa session | PASS | Bản chạy lại kiểm tra status và ghi `remote_deleted_ack=true`; chỉ chứng minh DELETE nhận 2xx trong probe, không chứng minh task dừng hoặc dữ liệu/quota đã dọn |

| Lượt | Tên | Byte | SHA-256 |
| --- | --- | ---: | --- |
| 1 | `report.md` | 5125 | `80c37e6a688e9034acd4ec107dea61da082d0ece2516a036f4c14168ecb4b08f` |
| 2 | `report.md` | 6208 | `310cbf140214fcd0f1fbcc871114ad2c7d8cf98e3864b7261015be85a18bcb8c` |

## Gate Muse thật còn mở

| Gate | Trạng thái | Cần quan sát |
| --- | --- | --- |
| Task tìm hiểu → tóm tắt → viết báo cáo qua `cloud_worker` | NOT_RUN | Background Task thật, nội dung cuối đúng và sự kiện nghiên cứu |
| Hai Sub-Agent tách account/session context | NOT_RUN | Hai Agent Session, hai remote session; không lẫn history |
| Hai send trong cùng con ở Orchestrator | NOT_RUN | Cùng Continuation Key/account/session xuyên hai invocation |
| Thu hồi qua Artifact Store, cha đọc sau archive | NOT_RUN | Artifact IDs, scope, origin/MIME/size/SHA-256 và parent đọc đúng byte |
| Disconnect/reconnect lúc task đang chạy | NOT_RUN | Nối lại đúng session, không POST lại; session khác không bị đóng socket |
| Kill/restart giữa chừng | NOT_RUN | Cùng invocation/session/account/deadline, đếm POST trước/sau, không gửi trùng |
| Closer archive con và nhận DELETE 2xx | NOT_RUN | Provider probe chứng minh riêng một DELETE 2xx; cần chứng minh orchestration ghi `SESSION_ARCHIVED`, ACK xảy ra trước `remote_deleted_at` và retry lỗi xóa |
| History sau DELETE và VM cleanup | NOT_RUN | Session không còn truy cập, đo riêng task dừng và file còn/mất |
| Quota trước/sau và sau cleanup | NOT_RUN | Mốc quota có thời gian; phân biệt độ trễ cập nhật và task còn chạy |
| Approval refusal trên upstream thật | NOT_RUN | Loại approval, quyết định, `MUSE_TASK_REJECTED`, không ghi nội dung nhạy cảm |

Không có bằng chứng `FAIL` được ghi trong summary stage0; điều này không nâng bất kỳ gate `NOT_RUN` nào thành đạt. Cần bổ sung evidence của ticket live trước khi kết luận nghiệm thu tính năng.

## Source checks

Port, Host Tool, catalog/Admin và closer là các phần triển khai ở server, không sửa thư viện. Bằng chứng test/check có phạm vi riêng trong [implementation map](../../.scratch/muse-task-tool/map.md) và các ticket liên quan; trạng thái `resolved` của ticket tài liệu chỉ chứng minh đã bàn giao tài liệu. Chưa dùng kết quả VM giả để điền PASS vào các gate Muse thật phía trên.

Snapshot source evidence do agent tích hợp cung cấp trong phiên triển khai: Host Tool 15 tests + cargo check PASS; catalog/Admin HTTP/settings prompt/Web/typecheck PASS; gateway 29 test VM giả và broad Muse 80 PASS/2 ignored; closer 9 test xác định PASS gồm lifecycle/ACK retry, parent đọc artifact sau archive, pagination, journal hỏng, restart con completed và guard nhiều account/key. Restart con active và nghiệm thu live đầy đủ vẫn cần bằng chứng riêng. Đây là snapshot trên các nhánh triển khai trước tích hợp cuối, không phải kết quả chạy lại bởi ticket tài liệu.
