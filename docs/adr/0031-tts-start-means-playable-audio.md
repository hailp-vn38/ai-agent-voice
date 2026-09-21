# ADR 0031 — `tts:start` chỉ báo audio đã phát được

## Status
Accepted

Actor chỉ enqueue `tts:start` khi đã có AudioPacket hợp lệ đầu tiên, rồi mới enqueue audio đó. Final no-tool LLM text rỗng là `Failed(llm_empty_final_response)`; non-empty TTS input hoàn tất với zero valid AudioPacket là `Failed(tts_empty_audio)`. Cả hai không commit Delivered Assistant Response, không custom wire error, và chỉ gửi `tts:stop` nếu `tts:start` thực sự đã gửi.
