# ADR 0143 — Required Job Skill được nạp sẵn vào system prompt của Built-in Sub-Agent

Status: Accepted, 2026-11-12.
Extends: [ADR 0128](0128-admin-managed-job-skill-catalog.md).

Prompt của `wiki_ingestor` yêu cầu gọi `skill` cho ba Job Skill ngay lượt đầu của mọi Agent Run; `wiki_curator` làm tương tự với hai skill. Các skill này thực chất là một phần của prompt, nhưng lại đi qua cơ chế lựa chọn: Admin bỏ gán được (Sub-Agent sau đó bị từ chối khi nạp, không có cảnh báo lúc lưu), mỗi run tốn một lượt model để nạp, và nội dung mất khi Agent Session bị nén. Spec: [job-skill-catalog-hardening](../../.scratch/job-skill-catalog-hardening/spec.md).

Quyết định: phân biệt **Required Job Skill** với skill thường. Server đưa nội dung Required Job Skill vào system prompt của Sub-Agent; skill thường vẫn do Agent Session tự chọn qua tool `skill`.

## Decisions

- **Danh sách Required Job Skill cố định trong code, theo từng Built-in Sub-Agent.** `wiki_ingestor`: `wiki-ingest`, `wiki-taxonomy`, `wiki-page-format`. `wiki_curator`: `wiki-lint`, `wiki-query`. Sub-Agent khác không có. Tiêu chí: prompt mặc định nạp skill đó vô điều kiện.
- **Admin sửa được nội dung, không bỏ gán được.** `allowed_skills` thiếu Required Job Skill được server tự bổ sung, không bị từ chối.
- **Server ghép nội dung vào file prompt thành một khối do server sở hữu**, đặt sau prompt của Admin hoặc prompt mặc định, kèm câu nói rõ các skill đã nạp, phải làm theo trừ khi trái với prompt phía trên, và không cần gọi `skill` cho chúng.
- **Prompt đã lưu và prompt Admin thấy không chứa khối.** Bản chụp cũ của khối trong prompt đã lưu bị thay khi ghi file.
- **Khối được ghi lại lúc khởi động, khi lưu skill và khi lưu cấu hình Sub-Agent**, nên chỉnh sửa có hiệu lực cho run kế tiếp mà không cần khởi động lại.
- **Nội dung lấy từ bản trên đĩa nếu hợp lệ, nếu không thì từ bản nhúng.**
- **Required Job Skill vẫn nạp được qua tool `skill`.** Prompt Admin đã tuỳ chỉnh từ trước có thể còn câu yêu cầu nạp; từ chối lời gọi đó sẽ tái tạo đúng lỗi cần sửa.
- **Root Orchestrator và Custom Sub-Agent không có Required Job Skill.**
- **Prompt mặc định do `llm-wiki-rs` sở hữu bỏ bước nạp các skill này; `orchestrator-flow` tái xuất prompt đó.**
- **Không thay đổi orchestrator-agent.** Server đã sở hữu việc ghi file prompt.

## Considered Options

- **Giữ nguyên, chỉ cảnh báo khi Admin bỏ gán skill mà prompt nhắc tới:** phải dò ID skill trong văn bản prompt, vẫn tốn một lượt model mỗi run và vẫn mất skill sau khi nén.
- **Thêm khái niệm skill nạp sẵn vào orchestrator-agent:** tổng quát hơn nhưng cần phát hành thư viện, trong khi server đã ghép được ở lớp prompt. Có thể xét lại nếu host khác của thư viện cần.
- **Gộp hẳn nội dung skill vào prompt mặc định và bỏ các skill đó:** mất khả năng Admin sửa workflow tách khỏi prompt, mất cơ chế nâng cấp và khôi phục mặc định của Built-in Job Skill, và `wiki-taxonomy`, `wiki-page-format`, `wiki-query` còn được Sub-Agent khác dùng như skill thường.
- **Loại Required Job Skill khỏi danh sách nạp được:** catalog gọn hơn nhưng làm hỏng prompt Admin đã tuỳ chỉnh.
- **Cho Admin tự đánh dấu skill là bắt buộc:** chưa có nhu cầu; Custom Sub-Agent có thể viết thẳng hướng dẫn vào prompt của mình.

## Consequences

- System prompt của `wiki_ingestor` dài thêm khoảng 17 KB và của `wiki_curator` khoảng 7 KB ở mọi lượt. Phần này cố định giữa các lượt nên được provider cache; đổi lại mỗi run bớt một lượt model.
- Nội dung Job Skill do Admin sửa giờ nằm trong system prompt chứ không chỉ là kết quả tool. Thứ tự ưu tiên được nêu trong câu mở đầu khối, giống cách Wiki Ask đang làm với `wiki-chat`.
- Required Job Skill không còn phụ thuộc vào việc nén Agent Session.
