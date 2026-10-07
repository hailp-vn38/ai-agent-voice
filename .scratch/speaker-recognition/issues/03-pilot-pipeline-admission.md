# 03: Giới hạn toàn pipeline và báo busy trên WS

**What to build:** Operator bật envelope pilot tường minh; một Voice Session giữ quyền xử lý, các WS idle vẫn hoạt động và client nhận busy rõ ràng khi tranh capacity.

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] Deployment envelope áp dụng toàn process/mọi Agent/mode, không tự bật khi tạo provider hoặc đổi policy; acquire Voice Pipeline Processing Permit trước mở VAD/ASR.
- [x] Giữ qua Listening/Processing/Speaking và armed capture; barge-in cùng session reuse; release chỉ Ready/teardown sau writer terminal và native cleanup acknowledgement.
- [x] Text Detect và diagnostics dùng ASR/LLM/TTS cũng chịu gate hoặc chạy khi rảnh; không time slicing, không native work vượt cap, không queue vô hạn.
- [x] Pipeline status capability riêng, bounded state/reason, không owner identity; opt-in giữ WS/control và explicit retry; legacy busy close1013 kể cả Detect cần capacity.
- [x] Status gắn request/lifecycle nội bộ, stale output bị loại; busy không là auth/mismatch. Reference Client và wire public tests chứng minh contention, controls, abort, modes và cleanup; tài liệu client được cập nhật.

## Answer

Implemented explicit `[deployment].speaker_pilot` (default false), shared atomic
voice/enrollment/cold admission coordinator, capture-before-native pipeline ownership,
armed Auto/Realtime retention and same-session barge-in reuse. Ready/teardown release
waits writer terminal and physical provider admission/reset acknowledgement;
quarantined native work retains capacity until process restart. Text Detect and
VAD/ASR/LLM/TTS diagnostics share admission; remote Vision HTTP uses the same envelope.
Cold materialization and enrollment integration consume this coordinator in ticket 04.

`features.pipeline_status=true` receives bounded `pipeline/busy/capacity` while
preserving WS/control and requiring explicit retry. Legacy busy work starts close 1013.
Queued status is fenced by request epoch and session teardown. Reference Client crate
was intentionally retired; existing independent WS qualification helpers prove the
capability contract instead. See [configuration contract](../../../docs/04-configuration.md).

Validation: `cargo test -j2 -p voice-agent-server --test protocol_e2e --test vision_api`
passed all 11 tests, including contention, controls, text denial, cleanup retry,
armed Auto abort/disconnect, Auto/Realtime acoustic barge-in and Vision contention.
`cargo fmt --all --check` and `git diff --check` passed. No real-model qualification
or performance claim is made.
