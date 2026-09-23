# ADR 0010 — Không acoustic barge-in trong V1

## Status
Superseded by ADR-0045

V1 không dùng microphone audio hay VAD để interrupt khi Speaking, vì chưa có server AEC. Manual mode chỉ hủy qua `abort` hoặc `listen:start`; auto VAD chỉ endpoint speech khi Listening. Wake word/device-side interruption phải gửi `abort`; acoustic barge-in chỉ được xem lại cùng protocol timestamp mới và server AEC.
