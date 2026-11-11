# ADR 0142 — Một cách quét Skill Catalog và nâng cấp Built-in Job Skill theo Skill Base Version

Status: Accepted, 2026-11-12.
Amends: [ADR 0128](0128-admin-managed-job-skill-catalog.md).

Server và thư viện orchestrator-agent hiện mỗi bên tự quét thư mục skill, theo hai quy tắc khác nhau. Server dùng bản nhúng thay cho Built-in Job Skill hỏng trên đĩa; thư viện chỉ đọc đĩa, quét nghiêm ngặt lúc build và khoan dung lúc reload. Hệ quả: một mục hỏng trong thư mục skill làm server không khởi động được, và một Built-in Job Skill hỏng làm `reconfigure` thất bại, trái với ý định của ADR 0128. Ngoài ra Built-in Job Skill chỉ được ghi khi file chưa có, nên bản sửa phát hành cùng server không tới deployment đang chạy. Spec: [job-skill-catalog-hardening](../../.scratch/job-skill-catalog-hardening/spec.md).

Quyết định: thư viện là nơi duy nhất quyết định skill nào tồn tại, luôn ở chế độ khoan dung; server tự phục hồi và tự nâng cấp Built-in Job Skill trên đĩa trước khi thư viện quét.

## Decisions

- **Một hàm quét, do thư viện sở hữu.** Thư viện công khai hàm quét khoan dung trả về skill hợp lệ và mục bị bỏ qua kèm lý do. Build `Orchestrator`, `reload_skills` và mọi chỗ server cần biết Skill Catalog đều dùng hàm này.
- **Mục hỏng không bao giờ là lỗi khởi động.** Chỉ thư mục skill không tồn tại mới là lỗi cấu hình.
- **Skill lạ trong quyền của agent bị thu hẹp, không phải lỗi.** Áp dụng cho cả build và `reconfigure`. Kiểm tra ID khi Admin lưu cấu hình vẫn do server làm.
- **Server tự phục hồi Built-in Job Skill.** File thiếu hoặc không parse được được thay bằng bản mặc định lúc khởi động và trước mỗi lần ghi skill; nội dung bị thay được giữ ở file ẩn cạnh nó. Bản nhúng không còn đóng vai skill tồn tại khi đĩa hỏng.
- **Skill Base Version trong manifest ẩn ở gốc thư mục skill.** Mỗi Built-in Job Skill ghi hash nội dung của bản mặc định mà bản trên đĩa xuất phát từ đó.
- **Hash nội dung tính trên bản render chuẩn hoá**, không trên byte thô, để bản Admin khôi phục qua Admin Web trùng với bản mặc định.
- **Bản chưa sửa thì nâng cấp, bản đã sửa thì giữ.** Bản trên đĩa trùng Skill Base Version và khác bản mặc định hiện tại bị ghi đè lúc khởi động. Bản khác Skill Base Version không bao giờ bị đụng.
- **Deployment có trước manifest dùng danh sách hash cố định của các bản mặc định cũ**, sinh một lần từ lịch sử git. Bản không trùng hash nào được coi là Admin đã sửa.
- **Admin lưu một Built-in Job Skill thì Skill Base Version thành bản mặc định hiện tại.**
- **Admin API tách "đã sửa" khỏi "bản mặc định có cập nhật".** Cờ thứ hai đúng khi skill khác bản mặc định và Skill Base Version khác bản mặc định hiện tại.
- **Custom Job Skill hỏng hiện trong Admin API với trạng thái `invalid` và lý do**, xoá và ghi đè được bằng ETag của nội dung thô.

## Phần của ADR 0128 bị sửa

| ADR 0128 | ADR này |
|---|---|
| Built-in Job Skill được seed lúc khởi động khi chưa có | Thêm: bản chưa sửa được nâng cấp, bản hỏng được phục hồi |
| `SKILL.md` hỏng không chặn `reconfigure` nhờ tách `reload_skills` | Giữ ý định; bổ sung việc thu hẹp skill lạ để điều này đúng cả khi skill hỏng đang được gán |
| Không nói về lúc khởi động | Mục hỏng không chặn khởi động |

Các phần còn lại của ADR 0128 vẫn hiệu lực: `SKILL.md` trên đĩa là nguồn duy nhất, không có bảng SQLite, Built-in Job Skill không xoá được, mọi thay đổi cần ETag và đi qua `skill_write_lock`, run đang chạy giữ snapshot.

## Considered Options

- **Giữ Built-in Job Skill trong bản nhúng, đĩa chỉ chứa phần Admin ghi đè:** nâng cấp trở thành mặc nhiên và không cần manifest. Từ chối vì thư viện phải nhận skill từ hai nguồn, và ADR 0128 đã chọn thư mục skill là nguồn duy nhất mà Admin có thể mount và sửa trực tiếp.
- **Luôn ghi đè Built-in Job Skill lúc khởi động:** đơn giản nhất nhưng xoá mọi chỉnh sửa của Admin, trái với ADR 0128.
- **So sánh bằng byte thô:** bản Admin khôi phục qua Admin Web được render lại nên khác byte với bản nhúng, và sẽ bị coi là đã sửa mãi mãi.
- **Không có danh sách hash cũ, coi mọi bản khác mặc định trên deployment cũ là đã sửa:** không cần lịch sử git nhưng deployment hiện có không bao giờ nhận các bản sửa đã phát hành, đúng vấn đề cần giải quyết.
- **Giữ quét nghiêm ngặt lúc khởi động để phát hiện lỗi sớm:** lỗi vẫn được báo qua cảnh báo và trạng thái `invalid` trong Admin Web; cái giá của fail-fast ở đây là mất cả Voice vì một file hướng dẫn.
- **Server tiếp tục tự quét nhưng sửa cho giống thư viện:** hai bản cài đặt sẽ lại lệch nhau; đã lệch một lần.

## Consequences

- Cần phát hành orchestrator-agent trước khi server dùng được phần quét chung. Phần manifest và nâng cấp chỉ đụng server và làm trước được.
- Định dạng render chuẩn hoá trở thành một phần của hợp đồng: đổi nó làm mọi Built-in Job Skill bị coi là đã sửa.
- Admin sửa file trực tiếp trên đĩa vẫn được hỗ trợ; bản đó khác Skill Base Version nên được giữ khi nâng cấp.
