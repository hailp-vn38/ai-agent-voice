# Runbook Muse Task: nghiên cứu, file và phục hồi

Dùng cùng [báo cáo validation](muse-task-validation-2026-11-02.md). Mỗi lần chạy ghi commit, phiên bản thư viện, cấu hình giới hạn không chứa bí mật, thời gian UTC, tài khoản dạng nhãn, loại task, số POST, session/invocation được ánh xạ sang nhãn và PASS/FAIL/NOT_RUN theo từng phép đo. Chỉ giữ báo cáo đã lọc trong repo. Credential, key mã hóa, raw history/checkpoint và file nội dung riêng để ngoài repo; không in chúng vào terminal/log.

## Chuẩn bị

1. Dùng installation kiểm thử và kho Task/Artifact riêng. Không kill server đang phục vụ người dùng hoặc thay đổi DB production để dựng phép đo. Chụp quota trước khi chạy; xác nhận account đã được phép dùng và chi phí/quota dự kiến. Việc người dùng cho phép phiên chạy này không phải quyền tiêu quota cho các phiên sau.
2. Chuẩn bị Muse Gateway Account đã xác thực và model điều phối hỗ trợ tool calling. Bật `cloud_worker`, chọn `tool_providers.muse_task` theo [hướng dẫn settings](../muse-task-settings-upgrade.md). Giữ prompt an toàn, dùng Task nguồn Chat/API; không dùng Voice cho phép đo công việc dài.
3. Ghi cấu hình `artifact_max_bytes`, ngân sách task, giới hạn đồng thời và thời hạn tool. Mặc định tool tối đa hai lượt/account, `max_wait_secs=3600`, chấp nhận 60–7200 giây. Không mở approval allowlist chưa được quan sát/xác minh.
4. Chọn thư mục evidence ngoài repo và chỉ bật capture metadata cần thiết: thời gian, operation state, baseline, số user turn, POST count, stream event type, account/session alias, file metadata và cleanup HTTP status. Không log cookie, token, prompt riêng hoặc nội dung approval.

## Probe provider giai đoạn 0

Test ignored `muse_task_live_stage0_probe` ở `crates/vela-server/src/providers/llm/gateway/muse/tests/live_task.rs` dùng credential đã cấu hình của installation; không yêu cầu chép credential vào biến môi trường. Sau khi mã probe đã được tích hợp, chạy qua wrapper từ checkout kiểm thử:

```bash
MUSE_TASK_SPIKE_CONFIRM=spend-quota \
MUSE_TASK_SPIKE_REPO=/absolute/path/to/authorized-test-installation \
MUSE_TASK_SPIKE_ACCOUNT=authorized-account-key \
MUSE_TASK_SPIKE_OUTPUT_DIR=/absolute/path/outside-repo/evidence \
scripts/agent-test.sh run -- cargo test --quiet -p vela-server --lib \
  muse_task_live_stage0_probe -- --ignored --nocapture
```

Đọc chọn lọc log wrapper đã lưu nếu thất bại. Probe chỉ kiểm provider/VM với journal riêng; nó chưa thay thế các bước Background Task phía dưới. Client DELETE phải kiểm 2xx trước khi dùng trường success làm ACK.

## Nghiên cứu, nhiều lượt và file

1. Tạo Background Task yêu cầu `daily_root` giao một `cloud_worker` nghiên cứu tài liệu công khai, tổng hợp và viết `report.md` kèm nguồn. Yêu cầu con làm đủ hai lượt trước khi kết thúc: lượt đầu thu thập/tổng hợp, lượt hai kiểm chứng điểm thiếu và hoàn thiện báo cáo. Ghi loại stream `task.*`/browser event đã thực sự thấy; URL hoặc citations đơn lẻ không chứng minh web search.
2. Kiểm hai `send` thuộc cùng Agent Session, Continuation Key, Gateway Account và remote session; baseline tăng đúng lượt. Nếu đang chạy thì `MUSE_TASK_BUSY` hướng sang `collect`; `collect` không tăng POST prompt. Tạo con thứ hai và xác nhận remote session khác, không lẫn lịch sử.
3. Đo `output=message`, `markdown` và `files` riêng. Markdown luôn có artifact `.md`; phân biệt `origin=muse_file` và `origin=message`. Với files thiếu đính kèm, chỉ có một follow-up được journal, sau đó thiếu vẫn báo partial rõ ràng. Đối chiếu tên chuẩn hóa, MIME, byte, checksum và artifact ID; các định dạng được nhận gồm md/txt/pdf/csv/json/html/docx/xlsx/png/jpg/webp/mp4.
4. Dùng `shasum -a 256` trên file tải về bên ngoài repo để so checksum artifact. Kiểm cha đọc artifact qua quyền/scope thật cả trước và sau archive con; không coi đọc file trực tiếp từ VM là bước này. Kiểm message lớn được chuyển thành artifact và content vẫn có ID.

## Disconnect và kill/restart

1. Trong installation kiểm thử, tạo task đủ dài để có trạng thái Running được lưu. Đóng riêng kết nối transport của harness hoặc gây mất mạng có kiểm soát rồi khôi phục; không đóng socket dùng chung của account bằng thao tác fail-account giả. Đo reconnect đúng account/session và POST count không tăng. Nếu có session khác đang chạy, xác nhận nó tiếp tục hoạt động.
2. Lặp lại với tiến trình kiểm thử bị SIGKILL khi đã gửi nhưng chưa hoàn tất. Ghi PID chính xác và checkpoint đã flush trước khi kill; không dùng `pkill` toàn bộ server. Khởi động cùng binary/config, kho Task và credential keys; tiến trình cũ phải dừng trước khi tiến trình mới nhận lock.
3. So invocation, Continuation Key, account, remote session và deadline trước/sau restart. Đếm POST bằng capture transport/upstream đã lọc; history có hai user turn chưa đủ chứng minh không gửi trùng ở mọi cửa sổ. Thử cửa sổ mất ACK sau POST: phục hồi phải đối soát baseline trước khi gửi; không xác minh được thì `TOOL_OUTCOME_UNKNOWN`.
4. Khi chạy hết lát chờ, kiểm `TOOL_RETRY_LATER` giữ operation và nhả slot; retry dùng cùng invocation. Khi timeout/stalled, ghi partial text và `next_action=collect`; invocation collect mới không gửi prompt và không đặt lại ngân sách turn cũ. Trường hợp chưa kích hoạt được ghi NOT_RUN.

## Archive, cleanup và quota

1. Cho con hoàn tất rồi quan sát closer: trong chu kỳ quét khoảng 10 giây, Agent Session được archive, không có DELETE và phiên vẫn hiện trên web Muse. Gọi lại con phải trả `SESSION_ARCHIVED`, cha vẫn đọc artifact; follow-up sau đó tạo con mới. Các bước dọn phiên dưới đây chỉ áp dụng khi task yêu cầu xóa phiên, tức lời gọi `muse_task` có `delete_session_on_finish=true`; khi đó archive phải xảy ra trước DELETE.
2. Capture mã HTTP DELETE thật; chỉ 2xx mới là ACK. Kiểm `remote_deleted_at` chỉ xuất hiện sau ACK. Làm lỗi DELETE có kiểm soát để xác nhận chưa đánh dấu và lượt quét sau thử lại. Với session ID dùng trên nhiều account, mọi nhóm phải ACK trước khi đánh dấu; key xung đột phải hoãn cleanup.
3. Đọc history sau xóa và ghi đúng phản hồi. Đo riêng việc đang chạy có dừng không và file trong `tasks/<Agent Session ID>/` còn hay mất. Không suy ba kết quả đó từ DELETE ACK. Nếu còn task/file, ghi giới hạn và dọn qua giao diện upstream được hỗ trợ; không tuyên bố đã dọn VM khi chỉ mất session.
4. Chụp quota sau task, ngay sau cleanup và sau độ trễ cập nhật có ghi thời gian. Nếu quota tiếp tục giảm hoặc task còn chạy, ghi FAIL/giới hạn cụ thể và dừng chạy thêm trước khi điều tra. ACK không chứng minh hoàn quota.
5. Khởi động lại với con đang chạy rồi với con đã terminal: trường hợp đầu giữ phiên, trường hợp sau closer dọn ở lượt quét đầu. Đặt video cùng root để chứng minh phiên/cleanup tách biệt. Xác nhận con waiting/interrupted/retryable failure được giữ, terminal/budget/step limit được đóng và kết quả dở dang được báo đúng.

Kết thúc bằng bảng từng gate, bằng chứng đã lọc và việc cleanup còn nợ. Nếu task bị kill trước cleanup, dùng đúng account/session đã ghi để xác minh và dọn lại; không gửi task mới chỉ để thay thế bằng chứng còn thiếu.
