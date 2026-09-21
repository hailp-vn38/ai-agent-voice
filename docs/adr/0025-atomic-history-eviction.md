# ADR 0025 — History eviction theo Exchange Atom cũ nhất

## Status
Accepted

V1 giới hạn history đồng thời theo message count và prompt budget tokens, nhưng eviction theo Exchange Atom cũ nhất thay vì từng message. System và current turn luôn giữ; tool call/result không tách; user-only cancelled/failed turn là atom bình thường theo tuổi. Tool result được sanitize/cap trước context và đánh dấu truncation, còn token estimator có thể conservative khi adapter không biết tokenizer.
