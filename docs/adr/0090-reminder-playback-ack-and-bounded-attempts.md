# ADR 0090 — Reminder hoàn tất bằng Playback ACK, retry bằng attempt bền vững

## Status

Proposed, 2026-10-12. Bổ sung delivery contract riêng cho Reminder; chưa triển khai hoặc nghiệm thu.

Reminder chỉ thành `completed` sau `reminder.played` từ client đã drain toàn bộ audio sink, được server xác thực và commit. Chọn ACK phía client thay vì lấy `tts:stop`/writer outcome làm phát xong: [ADR-0014](0014-dialogue-delivery-commit.md) và [ADR-0046](0046-long-lived-ws-turn-identity-and-writer-terminal-outcome.md) vẫn chỉ chứng minh server-side delivery của hội thoại.

- Chỉ claim khi có current capable session/capacity; revalidate Reminder pending, Device enabled, quyền và session trước playback. Offline/busy/enqueue failure trước bắt đầu phát không tiêu retry; claim chưa start được release/recover.
- Attempt có identity/lease và trạng thái persisted `claimed/played/failed/uncertain`; progress streaming chỉ transient. `attempts_used` là tổng bền vững trên Reminder, tăng đúng một lần cho attempt thực sự bắt đầu phát, không reset khi retry/restart.
- ACK phải khớp authenticated Device, current session instance, Reminder, attempt và lease; ACK trùng idempotent, session bị thay không terminalize attempt mới. Commit completed đóng attempt và tăng Schedule Revision atomically, rồi gửi `delivery_result` gắn attempt/revision và sync hint.
- Failure/abort/ACK timeout hoặc mất WS sau khi bắt đầu phát dùng failed/uncertain và bounded backoff; hết tổng attempts hoặc lateness grace thì expired và bump revision. ACK deadline theo duration/latency budget, có hard cap để utterance dài không bị retry sớm.

Không cam kết exactly-once playback: mất mạng sau phát trước ACK có thể gây phát trùng; dedupe và bounded retry chỉ giảm rủi ro. Playback ACK chứng minh phần mềm drain sink, không chứng minh âm vật lý. Mỗi attempt mới là một TTS operation mới; từng ASR/LLM/TTS/MCP operation vẫn không automatic retry theo [ADR-0023](0023-zero-provider-retries.md).

Nguồn: [Guide §2, §4.3, §5.3, §9.4, §13–16](../voice_reminders_direct_tts_implementation_guide.md#53-reminder-audio-delivery--playback-ack-không-prepareready-v1).
