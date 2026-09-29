# 02: Admin control-plane shell và Agent/Device CRUD

**What to build:** Admin có thể tạo, đọc, sửa và soft-disable Agent/Device qua authenticated Admin API bounded, với revision/audit behavior nhất quán và không cần trực tiếp sửa SQLite.

**Blocked by:** 01: SQLite foundation và boot lifecycle.

**Status:** completed

- [x] Admin router chỉ mount khi database và API đều enabled; đúng một Bearer header được constant-time compare, missing/malformed/wrong trả 401, disabled route trả 404, token/body/secret không bị log.
- [x] Shared request ID, JSON transport boundary, typed pagination/filter/sort, common error mapping và database busy/pool/storage taxonomy hoạt động cho mọi Admin endpoint; không raw SQL expression, compressed body hay request body vượt bound.
- [x] Agent và Device CRUD dùng typed resource/query bounds, typed PATCH Absent/Set/Clear, immutable key/device identity, optimistic revision và soft-disable only.
- [x] Admin success mutation/audit metadata là một transaction; audit không chứa request/config/history/secret contents và V1 không có audit-read route.
- [x] HTTP tests ở router seam chứng minh auth, 404 disabled, 401 malformed auth, request IDs, content/body errors, revision conflict, typed query caps, audit rollback and no secret readback.

## Comments

- 2026-09-29: review đã chặn bản đầu vì thứ tự transport/auth, SQLite error taxonomy, optimistic-revision race và query/PATCH semantics.
- 2026-09-29: đã sửa toàn bộ các điểm trên; transport gate chạy trước auth, update kiểm tra `rows_affected()`, sort/page-size được allowlist/cap và thực thi, Device Clear bị reject đúng kiểu, lỗi busy được map riêng, và router tests đã bổ sung negative coverage.
- 2026-09-29: bằng chứng hiện tại: `cargo check -p voice-agent-server`, `cargo test -p voice-agent-server --test admin_api`, `cargo test -p voice-agent-server --test database_bootstrap` và `git diff --check` đạt. `cargo test --workspace` chạy qua các test chính nhưng còn 3 failure trong `speechoutput_tracer` (`complete_vietnamese_sentences_announce_once_before_llm_eof`, `public_websocket_sends_each_complete_llm_sentence_once`, `full_segment_capacity_pauses_llm_until_all_segments_are_spoken`).
- Verification: `cargo check -p voice-agent-server`, `cargo test -p voice-agent-server --test admin_api --test database_bootstrap`, `cargo fmt --check`, `git diff --check` đạt. Full workspace còn 3 failure có sẵn trong `speechoutput_tracer`, ngoài phạm vi Admin API.
