# 02: Admin control-plane shell và Agent/Device CRUD

**What to build:** Admin có thể tạo, đọc, sửa và soft-disable Agent/Device qua authenticated Admin API bounded, với revision/audit behavior nhất quán và không cần trực tiếp sửa SQLite.

**Blocked by:** 01: SQLite foundation và boot lifecycle.

**Status:** ready-for-agent

- [ ] Admin router chỉ mount khi database và API đều enabled; đúng một Bearer header được constant-time compare, missing/malformed/wrong trả 401, disabled route trả 404, token/body/secret không bị log.
- [ ] Shared request ID, JSON transport boundary, typed pagination/filter/sort, common error mapping và database busy/pool/storage taxonomy hoạt động cho mọi Admin endpoint; không raw SQL expression, compressed body hay request body vượt bound.
- [ ] Agent và Device CRUD dùng typed resource/query bounds, typed PATCH Absent/Set/Clear, immutable key/device identity, optimistic revision và soft-disable only.
- [ ] Admin success mutation/audit metadata là một transaction; audit không chứa request/config/history/secret contents và V1 không có audit-read route.
- [ ] HTTP tests ở router seam chứng minh auth, 404 disabled, 401 malformed auth, request IDs, content/body errors, revision conflict, typed query caps, audit rollback and no secret readback.

## Comments

- 2026-09-29: implementation thử nghiệm đã bị review chặn: cần sửa thứ tự transport/auth, SQLite error taxonomy và optimistic-revision race trước khi ticket có thể hoàn tất.
- 2026-09-29: đã sửa transport gate trước auth, kiểm tra `rows_affected()` để chống lost update, và giữ taxonomy `database_busy` cho SQLite busy/locked ở các truy vấn list/get. Full workspace còn 3 test `speechoutput_tracer` flaky/unrelated fail; cần hoàn tất coverage taxonomy/rollback và query semantics trước khi đánh dấu done.
- 2026-09-29: bằng chứng hiện tại: `cargo check -p voice-agent-server`, `cargo test -p voice-agent-server --test admin_api`, `cargo test -p voice-agent-server --test database_bootstrap` và `git diff --check` đạt. `cargo test --workspace` chạy qua các test chính nhưng còn 3 failure trong `speechoutput_tracer` (`complete_vietnamese_sentences_announce_once_before_llm_eof`, `public_websocket_sends_each_complete_llm_sentence_once`, `full_segment_capacity_pauses_llm_until_all_segments_are_spoken`).
- Còn lại trước khi resolve: áp dụng `sort`/page-size bounded thực sự, thống nhất `Patch::Clear` cho Device, map busy/pool/storage cho mọi CRUD path, kiểm tra audit rollback/no-secret-readback và bổ sung race/transport/query negative coverage.
