# 05 — Development workflow và maintainability

## 1. Thứ tự triển khai

1. Protocol DTO + serde tests.
2. OTA + WS handshake.
3. Single writer + SessionActor.
4. Opus decode/encode round-trip.
5. Manual listen flow.
6. VAD flow.
7. ASR provider contract.
8. LLM stream + sentence segmenter.
9. TTS + pacer.
10. Abort/generation cancellation.
11. Device MCP.
12. E2E tests + deployment.

## 2. Mỗi feature phải có

- Tài liệu flow hoặc cập nhật flow hiện có.
- Unit test cho pure logic.
- Integration/contract test cho boundary.
- Log field đủ để debug: `session_id`, `device_id`, `generation`, `provider`.
- Không thêm dependency ngược layer.

## 3. Commit strategy

Mỗi commit nên build/test được và tập trung một concern. Ví dụ:

```text
docs: define websocket protocol contract
feat(protocol): add client/server message types
test(protocol): add hello/listen fixtures
feat(transport): add websocket upgrade and single writer
feat(session): add session actor and generation lifecycle
```

Không gộp refactor + feature + formatting lớn vào cùng commit.

## 4. Khi sửa bug

Trình tự bắt buộc:

1. Viết test tái hiện bug.
2. Xác nhận test fail.
3. Sửa ở module sở hữu invariant.
4. Chạy test module.
5. Chạy regression test các boundary liên quan.
6. Chạy E2E nếu bug chạm flow voice.

Không fix bằng cách thêm condition tạm ở module khác nếu invariant thuộc module hiện tại.

## 5. CI tối thiểu

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Optional live tests chỉ chạy khi có secret và flag explicit.

## 6. Observability

Log structured bằng `tracing`.

Ví dụ fields:

```text
session_id
client_id
device_id
generation
phase
provider
latency_ms
queue_depth
```

Các latency nên đo:

- utterance end -> ASR final
- ASR final -> LLM first token
- LLM first token -> TTS first audio
- TTS first audio -> first Opus sent
- total turn latency
