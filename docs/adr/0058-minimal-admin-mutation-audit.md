# ADR 0058 — Admin mutation audit tối thiểu, không chứa dữ liệu nhạy cảm

## Status

Accepted

`admin_audit_events` ghi metadata bounded cho mutation thành công và authenticated revision conflict: server-generated request correlation, resource/action, revision, outcome, coarse error kind và affected rows. Success audit insert nằm trong cùng transaction với mutation/purge, failure rollback mutation và trả `503`; conflict audit best-effort không thay `409`. Authentication failure chỉ telemetry; audit không lưu prompt, transcript, config JSON, secret reference/value, Authorization, body, diff hay arbitrary error. Audit maintenance batch/yield chạy initial rồi mỗi 24 giờ theo retention riêng `database.audit.retention_days`, không audit chính cleanup và failure không block runtime/mutation.
