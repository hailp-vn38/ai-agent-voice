# 03: Module web `useSystemLive()`

**What to build:** Composable `useSystemLive()` cùng transport SSE thật và transport giả cho test, theo interface trong spec. Chưa trang nào dùng nó ở ticket này; nó được kiểm thử độc lập qua interface.

**Blocked by:** 01 (hợp đồng snapshot và route).

Status: resolved

Spec: [system-live](../spec.md) · [ADR 0146](../../../docs/adr/0146-system-live-snapshot-stream.md)

- [x] Kiểu `SystemLiveSnapshot` trong `api/types/system.ts`, dùng lại `SystemResources` và `SystemStatus`.
- [x] Transport thật trong `api/system-live.ts`: fetch với Bearer, dùng `consumeSse`/`createSseParserState`, `handleSessionResponse` cho 401/403, trả `'ended' | 'unauthorized'`, ném lỗi khi mạng hỏng hoặc content-type sai.
- [x] `composables/useSystemLive.ts` trả `snapshot`, `cpuHistory`, `connection`, `freshness` (đều readonly).
- [x] Một kết nối mỗi tab dù có bao nhiêu caller; đóng 10 giây sau khi caller cuối rời.
- [x] Tab ẩn: abort ngay, `paused`, giữ snapshot. Tab hiện: mở lại ngay.
- [x] Backoff 1s → 30s; về 1s sau khi nhận snapshot. Stream kết thúc gọn cũng nối lại.
- [x] `unauthorized`: xoá snapshot và lịch sử CPU, ngừng nối lại cho tới khi có caller mới.
- [x] Độ tươi theo đồng hồ trình duyệt lúc nhận: `unavailable` / `stale` (quá 3 × chu kỳ hoặc không `live`) / `fresh`.
- [x] Lịch sử CPU tối đa 60 mẫu, chỉ thêm khi `sampled_at` mới.
- [x] Hàm cài transport chỉ dành cho test; không lộ transport qua `useSystemLive()`.
- [x] Test Vitest với fake timers và transport giả, theo danh sách trong spec; test transport thật theo mẫu `job-updates.test.ts`.
- [x] Chạy test qua `scripts/agent-test.sh`.

## Comments

### 2026-11-13

- Implemented and verified the SSE transport and shared composable with fake timers and transport.
- Targeted tests: `scripts/agent-test.sh run -- npm --prefix apps/admin-web run test -- --reporter=dot src/api/system-live.test.ts src/composables/useSystemLive.test.ts` — 2 files, 9 tests passed.
- Type check: `scripts/agent-test.sh web typecheck` — passed.
