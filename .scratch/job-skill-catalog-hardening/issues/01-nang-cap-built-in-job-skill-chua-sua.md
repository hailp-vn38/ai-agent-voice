# 01: Built-in Job Skill chưa sửa tự nhận bản mặc định mới

**What to build:** Sau khi nâng cấp server, Built-in Job Skill trên đĩa mà Admin chưa sửa được thay bằng bản mặc định mới lúc khởi động; skill Admin đã sửa giữ nguyên. Server ghi Skill Base Version của từng Built-in Job Skill vào một manifest ẩn ở gốc thư mục skill, so sánh bằng hash nội dung (SHA-256 của bản render chuẩn hoá). Deployment có trước manifest được nhận diện bằng danh sách hash cố định của các bản mặc định cũ, sinh một lần từ lịch sử git của các Built-in Job Skill.

Khi Admin lưu một Built-in Job Skill, Skill Base Version của nó thành bản mặc định hiện tại.

Chỉ đụng server. Xem spec: [Nâng cấp Built-in Job Skill](../spec.md#nâng-cấp-built-in-job-skill-adr-0142) và bảng quy tắc ở đó; [ADR 0142](../../../docs/adr/0142-single-skill-discovery-and-builtin-skill-upgrades.md).

**Blocked by:** không

**Status:** resolved

- [x] Thư mục skill có manifest và một Built-in Job Skill trùng Skill Base Version nhưng khác bản mặc định hiện tại: sau khi dựng app, file trên đĩa bằng bản mặc định hiện tại
- [x] Built-in Job Skill khác Skill Base Version đã ghi: sau khi dựng app, file không đổi một byte
- [x] Thư mục skill không có manifest, chứa đúng nội dung của một bản mặc định cũ trong lịch sử git: sau khi dựng app, file bằng bản mặc định hiện tại
- [x] Thư mục skill không có manifest, chứa nội dung không trùng bản mặc định nào: file không đổi
- [x] Admin khôi phục mặc định qua Admin API (nội dung được render lại) rồi bản mặc định thay đổi: lần dựng app sau skill được nâng cấp
- [x] Built-in Job Skill mới chưa có trên đĩa được ghi ra khi dựng app
- [x] Custom Job Skill không bao giờ bị đụng tới
- [x] Dựng app hai lần liên tiếp không làm thay đổi file nào ở lần thứ hai
- [x] Manifest không xuất hiện như một skill hay một file liên quan trong Admin API
