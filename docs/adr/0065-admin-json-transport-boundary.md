# ADR 0065 — Admin JSON Transport Boundary

## Status

Accepted

Admin mutation JSON dùng shared middleware/extractor seam với raw body limit 256 KiB. Request ID tạo trước; transport protection chạy trước auth/deserialization, rồi auth, content-type validation, JSON extraction và handler. Chỉ `Content-Type: application/json` với valid parameter được nhận; missing/sai là `400 invalid_content_type`, malformed JSON là `400 invalid_json`, oversized là `413 request_too_large`.

V1 chỉ nhận absent/`identity` Content-Encoding; mọi encoding khác là `415 unsupported_content_encoding`, không gắn decompression middleware trên Admin router. Boundary không parse, truncate hoặc log content khi reject. Resource-specific limits như Provider config vẫn áp thêm sau JSON extraction.
