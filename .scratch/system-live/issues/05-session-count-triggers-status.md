# 05: Số Voice Session đang hoạt động thay đổi thì System Status được tổng hợp lại sớm

**What to build:** `SystemLive` tổng hợp lại System Status khi số Voice Session đang hoạt động thay đổi, không đợi tới tick 15 giây. Có debounce để một loạt thay đổi chỉ tạo một lần tổng hợp.

**Blocked by:** 01.

Status: needs-triage

Spec: [system-live](../spec.md) · [ADR 0146](../../../docs/adr/0146-system-live-snapshot-stream.md)

- [ ] Nghe `watch` số session đang hoạt động sẵn có trong `lifecycle` (drain).
- [ ] Debounce 1 giây; vẫn giữ quy tắc chỉ tổng hợp khi có subscriber và không có hai lần tổng hợp chồng nhau.
- [ ] Tổng số lần tổng hợp vẫn có trần: không quá một lần mỗi giây.
- [ ] Test với tokio time paused: 10 thay đổi trong 500ms → một lần tổng hợp; không subscriber → không tổng hợp.
- [ ] Chạy test qua `scripts/agent-test.sh`.

Ghi chú triage: chỉ làm nếu sau ticket 04 Admin thấy độ trễ 15 giây của số session là vấn đề.

## Comments
