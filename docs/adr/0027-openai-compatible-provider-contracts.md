# ADR 0027 — OpenAI-compatible LLM/TTS adapter concrete cho V1

## Status
Accepted

V1 dùng `openai_chat_completions_v1` (`POST /v1/chat/completions`, SSE function tool calls) và `openai_speech_v1` (`POST /v1/audio/speech`, JSON request, WAV response). Những adapter này convert ở provider boundary thành domain event; fixture contract được pin theo wire shape này, còn vendor endpoint tương thích thay qua config. ASR không còn thuộc ADR này: Phase 3 dùng local streaming ASR theo ADR-0039.
