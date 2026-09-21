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
- stream error giữa câu.
- cancellation giữa stream.
- fragmented tool-call arguments.
- tool result -> second LLM request.
- max tool depth.
- no tools khi MCP chưa ready.
- tool-capable round có prose trước tool call -> prose không tới SpeechOutput.
- final round sau tool result không có tool call -> buffered text được đưa vào SpeechOutput.

## Command

```bash
./scripts/test-module.sh llm
```
