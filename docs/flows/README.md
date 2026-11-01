# Flow index

Task nền hiện tại dùng [ADR 0127](../adr/0127-orchestrator-flow-task-and-wiki-runtime.md), [Flow 16](16-task-flow-multi-kind-artifacts.md) và [Web Timeline](09-agent-job-web-timeline.md). Các flow Plugin V3 bên dưới là lịch sử của runtime đã thay thế. Flow 08 Hybrid và Flow 10 OpenCode đã được loại bỏ.

1. [WebSocket](01-websocket.md)
2. [VAD](02-vad.md)
3. [ASR](03-asr.md)
4. [LLM](04-llm.md)
5. [TTS](05-tts.md)
6. [Interrupt / cancellation](06-interrupt-cancellation.md)
7. [Device MCP](07-device-mcp.md)
9. [Background Task — Web Timeline](09-agent-job-web-timeline.md)
11. [Core Plugin Activation V3](11-core-plugin-activation-v3.md)
12. [Plugin Settings & Manual Operation V3](12-plugin-settings-and-manual-operation-v3.md)
13. [Knowledge + Memory V3 — Admin API import, draft and publish](13-knowledge-v3.md)

Mỗi flow mô tả boundary, sequence, invariants và test contract. Khi hành vi runtime thay đổi, tài liệu flow tương ứng phải được cập nhật cùng PR.

13. [Core TTS Provider → WAV Job Artifact V3](13-core-tts-v3.md)
15. [Core ASR — Job-owned WAV to final text V3](15-core-asr-v3.md)
16. [Unified Task Flow (`orchestrator_flow_rs`) & Multi-Kind Task Artifacts](16-task-flow-multi-kind-artifacts.md)
17. [Thu thập nhiều nguồn và nạp vào Wiki](17-batch-source-collection.md)
