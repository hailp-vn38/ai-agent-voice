# 08: Chuyển test background, Studio Assistant và test trong crate (lô 2)

Status: resolved

**What to build:** Bước *migrate* thứ hai. Các test còn lại có gửi Bearer tới Admin API chuyển sang Admin Web Token, cùng cách với ticket 07.

Phạm vi lô này: bộ test background HTTP, bộ test web assistant, và các test nằm trong mã nguồn server có gọi Admin API (background, knowledge và Wiki Chat, Wiki Conversation, LLM Gateway, System MCP, notification hub, system logs).

Test trong mã nguồn không dùng được harness của test tích hợp; nếu cần, đưa helper lấy token về một chỗ cả hai bên dùng chung thay vì chép lại.

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] Không test nào trong phạm vi còn gửi giá trị `admin_token` làm Bearer
- [ ] Config của các test này có `jwt_secret` hợp lệ
- [ ] Chỉ có một helper lấy Admin Web Token cho test, không có bản chép
- [ ] Không kỳ vọng hành vi nào bị sửa ngoài các test kiểm chính việc xác thực
- [ ] Các test trong phạm vi qua bằng `scripts/agent-test.sh targeted`, rồi `package` cho crate server
