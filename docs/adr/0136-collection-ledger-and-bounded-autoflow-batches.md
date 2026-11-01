# ADR 0136 — Collection Ledger and Bounded Auto-Flow Batches

Status: Accepted, 2026-11-01.
Extends: [ADR 0133](0133-durable-waiting-retry-and-job-budgets.md).

Một job nhiều video hoặc tài liệu không còn dựa vào context của LLM để biết việc nào đã xong. **Collection Ledger** trong `tasks.db` và frontmatter của `sources/` giữ tiến độ, còn Auto-Flow nhận từng lô giới hạn và cách ly nguồn lỗi.

## Decisions

- **Định danh nguồn nằm ở nội dung, không ở tên file.** `add_to_sources` nhận ra video theo `video_id` và tài liệu theo `content_sha256`. Lưu lại là no-op có báo cáo; trùng tên nhưng khác định danh thì lưu file riêng. Chỉ `force=true` mới ghi lại.
- **`sources/` chỉ chứa nội dung gốc.** Việc phân tích diễn ra một lần trong `wiki_ingestor`.
- **Session con mới cho mỗi lô thay cho "dọn context".** Agent không đo hay dọn được context; session con kết thúc thì transcript không đi theo agent cha. Danh sách việc còn lại đọc qua `collection_status`.
- **Chạm step cap không phải là hoàn thành.** Run vẫn `completed` và có bản tóm tắt, kèm mã `STEP_LIMIT_REACHED`; Task thành `stopped`; cha thấy `incomplete="true"`.
- **Giới hạn retry tính theo chuỗi không tiến độ.** `max_resume_attempts` đếm các attempt liên tiếp không tạo được step mới. Turn có tiến độ được chặn bởi ngân sách của ADR 0133.
- **Một trang tóm tắt chưa phải là ingest xong.** Nguồn còn dở khi trang tóm tắt nhắc tới entity/concept chưa tồn tại, hoặc nguồn mới hơn trang tóm tắt.
- **Auto-Flow chờ thu thập xong, tối đa 2 giờ.** Nó nhận tối đa 10 nguồn mỗi chặng và chỉ tổng hợp khi hàng đợi hết.
- **Nguồn lỗi 3 chặng thì bị cách ly.** Nguồn đó không chặn hàng đợi hay chặng tổng hợp. Người vận hành xem và giải phóng qua API; nguồn tự ra khỏi cách ly khi đã đủ trang.
- **Tool ghi không rõ kết quả có đường gỡ.** Admin ghi kết luận đã xác minh qua API; tool không bao giờ tự chạy lại.

## Considered Options

- **Skill nhắc agent tự kiểm tra và dọn context:** từ chối vì không có công cụ để làm và skill chỉ là lời nhắc. Skill vẫn được giữ để mô tả cách xử lý một lô.
- **Worker xác định không dùng LLM cho toàn bộ playlist:** để sau. Agent vẫn phải chọn `<domain>/<topic>`, và sổ theo dõi đã giải quyết được việc làm tiếp.
- **Giữ nguyên `Completed` khi chạm step cap:** từ chối vì đây là báo thành công giả.

## Consequences

- Thay đổi hành vi: tên file video mới có dạng `<slug>-<video_id>.md`; Task chạm step cap hiện là `stopped`; `fetch_playlist` và `fetch_transcript` không còn là API công khai.
- Một nguồn không có tiến độ có thể tốn tối đa 3 chặng LLM trước khi bị cách ly, thay vì kẹt im lặng.
- Source cũ không có `content_sha256` chỉ được nhận ra theo `video_id`.
- Chưa có slot riêng cho Task Voice; một job dài vẫn có thể làm Task Voice phải xếp hàng.
- Mọi Task không phải Voice đang chạy đều làm Auto-Flow chờ, kể cả Task không thu thập gì. Hệ thống không biết trước Task nào sẽ ghi `sources/`, nên chấp nhận độ trễ này. Thời gian chờ tính theo từng chu kỳ trong bộ nhớ và đếm lại sau restart.
