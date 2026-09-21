# ADR 0040 — ASR worker pool pin streaming recognition

## Status
Accepted

Phase 3 dùng pool bounded có `max_asr_streams` executor; `AsrStreamLease` pin một recognition stream vào đúng một worker từ Open qua Push đến Finish hoặc Cancel. Worker sở hữu recognizer và mutable stream state, còn SessionActor chỉ giữ opaque handle/lease và routing bằng session, generation, stream identity. Đây không phải generic job pool: migration audio frame giữa worker sẽ phá state streaming.

Cancel chỉ yêu cầu cleanup; slot chỉ release sau `Cancelled { session, generation, stream }` acknowledgement của worker, kể cả generation đó đã stale. Nếu quá cleanup grace mà không acknowledge, slot bị quarantine và affected Voice Session fail closed; không tái sử dụng worker mù hoặc tạo replacement vô hạn.
