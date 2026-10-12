# 07: Reminder đến hạn phát online và hoàn tất bằng Playback ACK

**What to build:** Admin tạo lịch, production scheduler phát trên WS đang mở, nhận sink-drain ACK rồi UI thấy completed và schedule mới.

**Blocked by:** 06: Đồng bộ lịch qua Voice WS và hiển thị Cache ACK.

Status: ready-for-agent

- [ ] Production bootstrap đúng một lifecycle-owned scheduler/process, scan bounded 1 giây và mutation/reconnect signals; router clones không thêm scanner, shutdown dừng claim/drain/join.
- [ ] Due eligibility/atomic lease chỉ khi current capable session/capacity; offline/busy/enqueue failure trước start không tăng attempts; cancel/Device disable/quyền/session revalidate trước phát.
- [ ] Attempt persisted identity/session/lease, tổng attempts_used tăng đúng một lần tại actual start; không LLM lúc due, effective TTS/permit/writer/generation dùng chung.
- [ ] Reminder FIFO due_at/id tối đa 16 queued, một active speech turn/Device, safe Ready/Listening/Processing/Speaking boundary và abort/security priority.
- [ ] Start/stop bind Reminder/attempt; played chỉ nhận sau sink drain trong honest harness, đúng Device/current session/active attempt/lease; duplicate ACK idempotent, old session reject.
- [ ] Completed + close attempt + revision atomic, sau commit delivery_result và sync hint; UI thể hiện waiting/completed/uncertain chính xác, firmware contract đợi terminal snapshot ACK.
- [ ] Basic failure/timeout/disconnect terminalize an toàn, không fake completed hoặc lease treo; restart cleanup bảo toàn identity. Ticket 08 hoàn thiện bounded retry/recovery policy.
- [ ] HTTP + production WS harness chứng minh một due và năm same-time due tuần tự/không interleave, cancel-before-play, stop khác played, terminal sync/replacement/backpressure.
