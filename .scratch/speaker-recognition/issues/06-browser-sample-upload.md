# 06: Thu WAV trên web và lưu sample hợp lệ

**What to build:** Admin ghi một sample bằng microphone browser và thấy slot accepted hoặc lỗi quality có thể sửa trong draft.

**Blocked by:** 04: Cold preparation dùng admission nguyên tử, 05: Quản lý Speaker và draft enrollment.

**Status:** ready-for-agent

- [ ] Recorder AudioWorklet/downmix/resample tạo thật PCM16 mono16k; clips5–10s, auto-stop10s, bounded RAM; HTTPS/localhost requirements và CORS hiện có rõ ràng.
- [ ] Raw WAV route cap512KiB toàn body/chunked, parser cap12s, reject encoding/format giả; JSON cap256KiB không thay. Reserve bounded upload/native enrollment capacity.
- [ ] Preliminary Calibration deployment profile và pinned preprocessing cung cấp quality/window rules; native inference qua exact manager/worker, không giữ transaction khi compute.
- [ ] Slot1–5 upload sequential CAS; quality fail không mutate; replace/remove làm stale validation mất hiệu lực. Bounded metadata/embedding storage, không raw audio retention.
- [ ] Cleanup tracks/nodes/AudioContext/fetch/object URLs khi cancel/navigation; không browser persistence. Browser encoding tests + HTTP WAV/mutation/cancel/timeout tests cùng slice.
