# 09: Cấu hình Agent policy và Template grants

**What to build:** Admin quản lý Off/Observe và quyền Speaker theo Template trên Agent, thấy dependencies rõ ràng; Required chưa thể bật khi thiếu gate/qualification.

**Blocked by:** 02: Bind Speaker Provider tùy chọn vào Template, 05: Quản lý Speaker và draft enrollment.

**Status:** ready-for-agent

- [ ] Policy revision riêng, absent Off contract nhất quán; grants replace-all explicit assigned/enabled Templates, không wildcard hoặc Client ID.
- [ ] Agent detail/Template surfaces phản ánh saved config, compatibility và required dependencies; thêm quyền chỉ admission mới, giảm quyền phát invalidation metadata.
- [ ] Candidate selection theo Agent enabled bindings và active provider space rồi mới kiểm Template grant; không lọc grant để ép nhận nhầm.
- [ ] Validation cấu trúc/revision/type/FK giữ nguyên; UI báo dependencies hiện biết và Required unavailable khi chưa có qualification. Không thêm runtime bypass hoặc đợi một ticket tương lai để hoàn tất slice Off/Observe.
- [ ] API public tests kiểm revision ownership, empty/wildcard/sai assignment, disabled resources, two tabs; Required enable guard fail closed cho tới khi15 hoàn thiện.
