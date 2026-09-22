# Testing 04 — LLM module

## Stream fixture

Mock SSE có thể chia câu thành nhiều delta không thuận tiện:

```text
"Xin"
" chào"
", tôi"
" có thể giúp bạn."
```

Assert segmenter không phụ thuộc token boundary.

## Cases

- text stream bình thường.
- punctuation segmentation.
- hard max chars flush.
- hard/soft punctuation respect `min_chars`/`soft_break_min_chars`; hard max split Unicode-safe, ưu tiên whitespace.
- stream error giữa câu.
- cancellation giữa stream.
- factory pin exact `llm` 1.3.8 với default features tắt, OpenAI + rustls TLS features.
- Phase 4 gọi `chat_stream_with_tools(messages, None)`.
- tool call bất thường sau một hoặc nhiều text delta -> `llm_unexpected_tool_call`, không MCP/retry, không submit segment mới và cancel SpeechOutput; audio đã delivered không bị diễn giải sai thành audio có thể thu hồi.
- global LLM concurrency fail-fast/controlled; cancellation hoặc timeout stop polling/drop stream, release permit sau task terminal; terminal event route full là controlled failure, không silent drop.
- startup OpenAI không tạo network request; DNS/auth/quota/model/5xx lỗi trong operation fail đúng generation, không làm server global unavailable.
- tool result -> second LLM request, max tool depth và prose buffering là contract Phase 6.

## Command

```bash
./scripts/test-module.sh llm
```
