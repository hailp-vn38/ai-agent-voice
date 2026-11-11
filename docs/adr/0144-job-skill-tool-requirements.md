# ADR 0144 — Job Skill khai báo tool nó cần, server cảnh báo khi Sub-Agent thiếu

Status: Proposed, 2026-11-11.
Extends: [ADR 0128](0128-admin-managed-job-skill-catalog.md).

Job Skill là văn bản hướng dẫn và không cấp quyền thực thi tool. Vì vậy một skill có thể được gán cho Sub-Agent không có tool mà skill bảo gọi, và không ai biết. Ví dụ hiện có: `wiki-query` hướng dẫn ghi synthesis và vá trang Wiki nhưng được gán mặc định cho `wiki_researcher`, vốn chỉ có tool đọc. Spec: [job-skill-catalog-hardening](../../.scratch/job-skill-catalog-hardening/spec.md).

Quyết định: mỗi Job Skill có thể khai báo **Skill Tool Requirement**. Server so khai báo đó với tool hiệu lực của Sub-Agent và báo khoảng trống cho Admin, không chặn.

## Decisions

- **Trường frontmatter tuỳ chọn `requires_tools`**, danh sách tên tool cách nhau bằng dấu phẩy. Tài liệu skill của orchestrator-agent mang trường này qua parse và render.
- **Danh sách rỗng không được render ra**, để tài liệu cũ giữ nguyên bản render và hash nội dung ([ADR 0142](0142-single-skill-discovery-and-builtin-skill-upgrades.md)).
- **Khai báo không cấp quyền và không chặn việc nạp skill.** Quyền tool vẫn chỉ đến từ cấu hình Sub-Agent.
- **Thư viện không kiểm tra tool có tồn tại.** Chỉ kiểm tra định dạng tên.
- **Khoảng trống tool là cảnh báo trong projection cấu hình Sub-Agent**: với mỗi skill trong `allowed_skills` hiệu lực, các tool trong `requires_tools` không có trong `allowed_tools` hiệu lực. Lưu cấu hình không bị từ chối.
- **`requires_tools` của Built-in Job Skill là tập tool tối thiểu chung** cho mọi Sub-Agent được gán mặc định. Hướng dẫn dùng tool mà chỉ một số Sub-Agent có được viết theo dạng có điều kiện.
- **Phân bổ mặc định phải không có khoảng trống**, được một test bất biến bảo vệ.
- **Tách `wiki-synthesis-filing` khỏi `wiki-query`** và gán mặc định cho `wiki_curator`; `wiki-query` chỉ còn hướng dẫn đọc.
- **Gợi ý skill cho Built-in Sub-Agent loại skill mà Sub-Agent đó không thể có đủ tool.**

## Considered Options

- **Từ chối lưu khi có khoảng trống:** làm hỏng cấu hình đã lưu của deployment hiện có và chặn việc Admin gỡ một tool. Cảnh báo đủ để Admin tự quyết.
- **Server giữ bảng tool cần cho từng Built-in Job Skill, không đụng frontmatter:** không cần phát hành thư viện, nhưng Custom Job Skill không được kiểm tra và khai báo nằm xa nội dung nó mô tả.
- **Tự suy ra tool cần bằng cách dò tên tool trong nội dung skill:** đã thử khi rà soát; cho nhiều kết quả sai vì skill dùng chung nhắc tới tool theo điều kiện.
- **Skill tự cấp tool nó cần:** trái với định nghĩa Job Skill trong glossary và mở đường cho nội dung do Admin sửa nới rộng quyền của Sub-Agent.
- **Lọc Skill Catalog của Sub-Agent theo tool:** ẩn skill thiếu tool khỏi `<available_skills>` thì an toàn hơn cảnh báo, nhưng một skill thường vẫn hữu ích khi thiếu một tool phụ, và Admin mất khả năng thấy vì sao skill không được dùng.

## Consequences

- Cần phát hành orchestrator-agent để frontmatter mang trường mới; có thể chung một phiên bản với ADR 0142.
- Deployment mà Admin đã ghi đè `allowed_skills` của `wiki_curator` không tự nhận `wiki-synthesis-filing`; skill này hiện trong phần gợi ý.
- Người viết Built-in Job Skill dùng chung phải giữ phần tool riêng của từng Sub-Agent ở dạng có điều kiện, nếu không test bất biến sẽ không qua.
