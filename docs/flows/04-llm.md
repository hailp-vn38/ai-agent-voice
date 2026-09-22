# Flow 04 — Streaming LLM

## 1. Input

```rust
pub struct LlmRequest {
    pub generation: u64,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
}
```

Adapter concrete V1 là `openai`, build qua crate Rust `llm`: factory map typed config sang `LLMBackend::OpenAI` và `LLMBuilder`, rồi bridge `ChatMessage`, `chat_stream_with_tools(...)` và `StreamChunk` thành `LlmEvent`. SessionActor không import type của crate `llm` hay parse OpenAI JSON/SSE.

`LlmRuntime` application-owned giữ provider shared, global semaphore và bounded route. Mỗi request là Tokio task theo Voice Session/generation, giữ permit từ runtime accept tới terminal event; timeout không reset bởi text delta. CancellationToken dừng polling/drop stream; terminal event không route được phải biến thành controlled failure, không silently drop.

Startup chỉ validate typed OpenAI config và build provider locally; không thực hiện network probe. DNS/TLS/auth/quota/model/5xx hay stream failure là `LlmEvent::Failed` của operation hiện tại, không làm server unavailable toàn cục.

## 2. Flow không tool call

```mermaid
sequenceDiagram
    participant A as SessionActor
    participant L as LlmProvider
    participant SEG as SentenceSegmenter
    participant T as TTS

    A->>L: stream(messages, tools)
    loop delta
      L-->>A: LlmDelta
      A->>SEG: push(delta)
      opt complete speakable segment
        SEG-->>A: segment
        A->>T: synthesize(segment, generation)
      end
    end
    L-->>A: Done
    A->>SEG: flush()
    A->>T: final segment if any
```

## 3. Sentence segmentation

Không gửi từng token vào TTS.

Segmenter nên ưu tiên:

1. dấu kết câu `. ! ? 。！？`;
2. dấu ngắt `, ; : ，；：` sau khi vượt minimum chars;
3. hard max chars để tránh đợi quá lâu.

Policy thuộc `[speech_output]`: `min_chars`, `soft_break_min_chars`, `max_chars`, với `1 <= min_chars <= soft_break_min_chars <= max_chars`. Punctuation V1 cố định: hard `. ! ? 。！？`, soft `, ; : ，；：`. Hard punctuation chỉ flush khi buffer đạt `min_chars`; soft punctuation chỉ flush khi đạt `soft_break_min_chars`. Tới `max_chars`, split Unicode-safe, ưu tiên whitespace gần ngưỡng rồi mới split tại character boundary. Pure function/state machine này phải có unit test riêng.

## 4. Tool call

Phase 4 gọi `chat_stream_with_tools(messages, None)` để dùng một typed streaming seam. Nếu stream vẫn phát tool call, actor fail generation là `llm_unexpected_tool_call`, cancel LLM và SpeechOutput, không submit segment mới, không gọi MCP và không retry. Audio đã deliver trước event không thể thu hồi; GenerationGate drop mọi audio stale/còn queue.

Phase 6 mới truyền `Some(&tools)` và áp dụng tool-result loop sau:

Nếu Device MCP ready, tool definitions được đưa vào request LLM.

```mermaid
sequenceDiagram
    participant L as LLM
    participant A as Actor
    participant MCP as Device MCP
    L-->>A: ToolCall(name,args)
    A->>MCP: tools/call
    MCP-->>A: result
    A->>L: continue with tool result
```

`max_tool_depth` giới hạn vòng lặp. Khi request có LLM-visible Tool, actor phải buffer prose và tool call đến hết round; không dựa vào prompt để bảo đảm thứ tự. Nếu round có tool call, prose round đó không đi vào TTS; server gọi MCP rồi bắt đầu round kế tiếp với tool result. Chỉ final round không tool call được đưa vào SpeechOutput. Request không có tool vẫn stream token → segmenter → TTS như bình thường.

Tool-level failure khi session khỏe được normalize thành tool result `ok:false` không chứa error body/secret rồi quay lại LLM. Khi tool depth vượt limit, inject `tool_depth_exceeded` và chỉ cho một final no-tools round; terminal session/cancellation failure không tiếp tục LLM.

Nhiều tool call của một round chạy tuần tự theo thứ tự LLM trả về, mỗi call có timeout riêng. Tool-level error không ngăn sibling call còn allowlisted khi session/generation vẫn khỏe; result giữ đúng tool_call_id và thứ tự để gửi vào round kế tiếp. `max_tool_depth` đếm tool round, không đếm individual call. Final no-tool round với text rỗng sau trim là `Failed(llm_empty_final_response)`, không gọi TTS.

## 5. History commit

Chỉ commit assistant response hoàn chỉnh vào dialogue khi turn kết thúc hợp lệ.

Nếu turn cancel giữa chừng:

- không ghi chunk dở như một assistant response hoàn chỉnh;
- optional: có thể lưu telemetry riêng.

## 6. Provider abstraction

V1 dùng OpenAI qua typed API của crate `llm` 1.3.8, pin exact với default features tắt và chỉ `openai`/`rustls-tls`. Adapter phải bridge stream thành event chuẩn:

```rust
pub enum LlmEvent {
    TextDelta(String),
    ToolCallDelta(ToolCallDelta),
    Usage(TokenUsage),
    Done,
}
```

V1 không automatic retry logical LLM operation sau timeout/lỗi.

## 7. Test contract

- multiple text deltas -> đúng concatenation.
- segmenter phát segment trước stream done.
- tool call fragmented JSON được assemble đúng.
- max tool depth được enforce.
- tool-capable round có prose + tool call -> prose không được TTS; final no-tool round mới được synthesize.
- cancellation dừng downstream TTS request.
- stale generation delta bị drop.
- provider stream error -> actor kết thúc turn sạch sẽ.
