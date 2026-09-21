# ADR 0003 — Provider traits ở boundary AI

## Status
Accepted

## Decision
ASR, LLM, TTS và VAD dùng contract ổn định; implementation vendor nằm ngoài session core.

## Consequences
Thay provider không sửa state machine. Provider-specific JSON không được leak vào core.
