# ADR 0048 — Persistent Transcript là opt-in, có retention

## Status

Accepted

`Persistent Transcript` mặc định tắt bằng `database.history.enabled = false`; khi bật, chỉ persist final user text và Delivered Assistant Response sau `WriterTurnOutcome::Normal`. Capture policy độc lập archive lifecycle: khi capture tắt, history admin read/purge và retention vẫn hoạt động nếu database/admin API enabled. `retention_days` bắt buộc trong `1..=365`, dùng Unix milliseconds UTC, cleanup `created_at < now_utc - retention_days` lúc startup rồi mỗi 24 giờ ngoài realtime path, kể cả khi capture mới đã tắt. HistoryWriter dùng `try_send`; queue full, writer closed hay database error drop record riêng lẻ với metric, không ảnh hưởng DialogueHistory, prompt, tool continuation hay turn outcome. Admin API bắt buộc auth riêng. Điều này supersede riêng phần cấm persistent transcript của ADR-0024; vẫn không log transcript, prompt, tool data, audio hay secret.
