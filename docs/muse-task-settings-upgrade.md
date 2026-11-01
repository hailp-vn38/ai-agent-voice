# Bật Muse Task trên installation hiện hữu

`cloud_worker` là Built-in Sub-Agent của `daily_root`, mặc định tắt. Nó chỉ có
`muse_task`; không được cấp tool ghi Wiki, Memory hay điều khiển thiết bị.
Tính năng cần Muse provider qua LLM Gateway. `agent_task` là tag của catalog
Task, không phải tag cấu hình `LlmCapabilityTag` và không cần sửa cấu hình tag
provider đã lưu.

Khi server khởi động hoặc Admin lưu Background settings, server đọc
`agent_settings.json` trong `orchestrator.store_dir`, bổ sung định nghĩa
built-in mới từ mã nguồn và dựng lại các file trong `orchestrator.prompts_dir`.
Không cần xóa settings, reset toàn bộ agents hay chỉnh các file prompt sinh ra.
Các override đã lưu của Admin tiếp tục được giữ nguyên. Nếu trước đây chỉ sửa
file prompt sinh ra, chuyển phần tùy chỉnh sang trường System prompt trong Admin
trước khi nâng cấp: các file này không phải nguồn cấu hình bền vững.

1. Khởi động server mới rồi mở Admin → Background → `cloud_worker`.
2. Chọn provider Muse cho tool `muse_task` (tag `agent_task`). Cấu hình này lưu ở
   `agents.cloud_worker.tool_providers.muse_task`. Model điều phối/tool-calling
   vẫn có thể dùng worker chat provider khác.
3. Đọc prompt mặc định và cảnh báo dữ liệu gửi sang dịch vụ bên thứ ba. Giữ các
   quy tắc use-once, send/collect, dữ liệu ngoài không đáng tin, không tự gửi
   Memory/Wiki, không gửi thông tin nhạy cảm và không chạy Task nguồn Voice.
4. Bật agent rồi lưu. `daily_root` chỉ được khám phá/giao việc cho agent khi bật.
   Tắt rồi lưu để thu hồi quyền discovery. Reset defaults cũng trả về trạng thái tắt.

Server luôn làm mới khối `<active_subagents>` do hệ thống quản lý khi ghi prompt
root, giữ nguyên phần nội dung tùy chỉnh bên ngoài khối đó. Với prompt root đã tùy
chỉnh, giữ hướng dẫn không giao việc cloud dài cho Task nguồn Voice (Job Budget
năm phút), chỉ giao Task nguồn Chat/API. Sub-Agent dùng một lần: hỏi tiếp sau khi
kết thúc cần tạo Sub-Agent mới và truyền ngữ cảnh rõ ràng.

Việc bật cấu hình không chứng minh Muse thật đã qua gate. Trạng thái spike và
kiểm chứng runtime/live phải được đánh giá riêng theo spec/runbook Muse Task.
