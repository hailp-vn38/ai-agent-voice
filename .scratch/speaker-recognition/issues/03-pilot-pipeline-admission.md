# 03: Giới hạn toàn pipeline và báo busy trên WS

**What to build:** Operator bật envelope pilot tường minh; một Voice Session giữ quyền xử lý, các WS idle vẫn hoạt động và client nhận busy rõ ràng khi tranh capacity.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [ ] Deployment envelope áp dụng toàn process/mọi Agent/mode, không tự bật khi tạo provider hoặc đổi policy; acquire Voice Pipeline Processing Permit trước mở VAD/ASR.
- [ ] Giữ qua Listening/Processing/Speaking và armed capture; barge-in cùng session reuse; release chỉ Ready/teardown sau writer terminal và native cleanup acknowledgement.
- [ ] Text Detect và diagnostics dùng ASR/LLM/TTS cũng chịu gate hoặc chạy khi rảnh; không time slicing, không native work vượt cap, không queue vô hạn.
- [ ] Pipeline status capability riêng, bounded state/reason, không owner identity; opt-in giữ WS/control và explicit retry; legacy busy close1013 kể cả Detect cần capacity.
- [ ] Status gắn request/lifecycle nội bộ, stale output bị loại; busy không là auth/mismatch. Reference Client và wire public tests chứng minh contention, controls, abort, modes và cleanup; tài liệu client được cập nhật.
