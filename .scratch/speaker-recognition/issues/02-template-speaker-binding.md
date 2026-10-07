# 02: Bind Speaker Provider tùy chọn vào Template

**What to build:** Admin chọn hoặc unlink Speaker Provider trên Template, giữ nguyên core fallback và default assignment của các Template hiện có.

**Blocked by:** 01: Tạo và kiểm tra Speaker Provider CAM++.

**Status:** ready-for-agent

- [ ] UI/API/DB/runtime-profile resolve hỗ trợ optional Speaker slot đúng type; không speaker default hoặc implicit fallback.
- [ ] Core absent bindings giữ server fallback; explicit broken core binding vẫn fail; first assignment không dùng count bốn-slot sai khi thêm Speaker.
- [ ] Bind/unlink CAS Template revision; provider usage/conditional deletion đúng; binding cold lưu desired, không build trong transaction.
- [ ] Public API/profile regression kiểm partial core, core + Speaker, sai type, assignment đầu, unlink và no-restart semantics; cập nhật Template UI/API documentation.
