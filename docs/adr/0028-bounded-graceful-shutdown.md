# ADR 0028 — Shutdown có cleanup control trước root termination

## Status
Accepted

V1 load/validate config một lần và không hot reload. SIGINT/SIGTERM chuyển app sang Draining: dừng accept, actor cancel turn, invalidate stale audio, gửi `tts:stop` nếu cần rồi WS close 1001, chờ grace 5 giây bounded và force-abort phần còn lại. Không cancel root token trước cleanup writer tối thiểu, không chờ AI hoàn tất và không persist dialogue.
