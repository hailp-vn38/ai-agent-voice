# ADR 0041 — VAD pin theo Auto cycle và reset có acknowledgement

## Status
Accepted

Auto Listening pin VadSession/worker xuyên nhiều utterance; sau Phase 3 terminal, actor gửi Reset và chỉ re-arm Listening sau `ResetDone`, để tail Push cũ không xen với utterance mới. Close mới release slot sau `Closed` acknowledgement; cleanup timeout quarantine worker. Manual không acquire VAD worker.

VAD inference failure, Reset failure/timeout hay Closed timeout là session-scoped fatal provider condition: invalidate generation, cancel dependent ASR, quarantine worker và close affected Voice Session 1011. Không fallback silent sang Manual hoặc làm chết toàn server.
