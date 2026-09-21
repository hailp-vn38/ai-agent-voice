# ADR 0017 — Active Turn giữ capacity từ ASR đến Drained

## Status
Accepted

Session try-acquire global Active Turn permit ngay sau khi utterance hoàn tất và trước ASR; nếu không có permit, turn bị từ chối ngay, không xếp chờ. Permit giữ xuyên ASR, LLM, MCP và SpeechOutput đến `Drained`, rồi được release ở Completed, Cancelled hoặc Failed; semaphore provider vẫn là giới hạn riêng.
