# Số liệu nền cho Task Playlist

Ngày đo: 2026-11-03. Mục đích: ghi nhận mẫu hiện có trước khi Task Recipe playlist được phát hành.

## Kết quả

DB `data/tasks-store/tasks.db` có 11 Task. Dùng các Task có `title` hoặc `prompt` chứa `playlist` hoặc `youtube` (không phân biệt hoa thường) làm cohort thăm dò thì có 5 Task, trong đó 4 hoàn tất:

- Tỷ lệ hoàn tất: **4/5 (80%)**.
- Số lượt LLM trung bình mỗi Task: **2.20**.
- Số bước Agent trung bình mỗi Task: **6.60**.
- Task có mã lỗi `STEP_LIMIT_REACHED`: **0/5 (0%)**; lượt Agent có mã lỗi này: **0**.

## Phương pháp

Chỉ chạy truy vấn tổng hợp trên `data/tasks-store/tasks.db` và `data/orchestrator-store/orchestrator.db`; không in, sao chép hoặc lưu ID, prompt, câu trả lời hay nội dung hội thoại. Cohort được lọc theo chuỗi trong `tasks.title`/`tasks.prompt`. Từ `tasks.session_id`, truy vấn đệ quy các session con theo `sessions.parent_session_id`; lượt LLM là số cặp `(session_id, turn_id)` phân biệt trong `agent_runs` của session gốc và các session con. Trung bình tính trên cả 5 Task trong cohort. `STEP_LIMIT_REACHED` được đếm trên `tasks.error_code` và `agent_runs.error_code`.

## Giới hạn

Schema hiện tại chưa có trường recipe có cấu trúc. Do đó 5 Task được chọn bằng tìm chuỗi chỉ là cohort heuristic; có thể bỏ sót Task playlist được diễn đạt khác hoặc gồm nhầm Task có nhắc tới YouTube/playlist. Đây là baseline gần đúng trước khi có Task Recipe, không phải nhãn chính xác. Khi triển khai recipe, nên ghi nhận recipe ID có cấu trúc và xuất lại cùng các số tổng hợp để so sánh. Chỉ lưu aggregate; không đưa dữ liệu prompt/answer vào tài liệu.
