# ADR 0090 — Reminder hoàn tất bằng Playback ACK, retry bằng attempt bền vững

## Status

Accepted, 2026-10-26 (Proposed, 2026-10-12).

Reminder chỉ thành `completed` sau `reminder.played` từ client đã drain toàn bộ audio sink, được server xác thực và commit. Chọn ACK phía client thay vì lấy `tts:stop`/writer outcome làm phát xong: [ADR-0014](0014-dialogue-delivery-commit.md) và [ADR-0046](0046-long-lived-ws-turn-identity-and-writer-terminal-outcome.md) vẫn chỉ chứng minh server-side delivery của hội thoại.

- Claim theo 2 pha (`claim_next_due_for_device` tạo attempt `status = 'claimed'`, `started_at_ms = NULL` với lease ngắn 10 giây chưa trừ quota; sau khi `SessionActor` chấp nhận phát mới gọi `mark_attempt_started` kiểm tra lại nguyên tử `d.enabled = 1`, `r.status = 'pending'`, `r.attempts_used < 3`, gán `started_at_ms = now`, cộng `attempts_used + 1` và đặt `lease_until_ms` động theo thời lượng âm thanh ước tính). Offline/busy/enqueue failure trước khi bắt đầu phát gọi `release_unstarted_attempt` xóa claim mà không tiêu retry.
- Attempt có identity/lease, mốc `started_at_ms` và trạng thái persisted `claimed/played/failed/uncertain`; progress streaming chỉ transient. `attempts_used` là tổng bền vững trên Reminder (tối đa 3 lần), tăng đúng một lần cho attempt thực sự bắt đầu phát, không reset khi retry/restart. Reaper (`reap_expired_leases_and_overdue`) chỉ thu hồi các attempt đã hết hạn `lease_until_ms <= now`, đảm bảo không bao giờ làm hết hạn sớm attempt thứ 3 khi thiết bị đang phát chờ ACK.
- Các sự kiện inbound `ReminderEvent` (`sync_request`, `sync_ack`, `played`, `failed`) được đẩy qua bounded per-session worker (`mpsc::channel(8)`) để không chặn vòng lặp WebSocket `recv()`. ACK phải khớp authenticated Device, current session instance, Reminder, attempt và lease; ACK trùng idempotent, session bị thay không terminalize attempt mới. Commit completed đóng attempt và tăng Schedule Revision atomically, rồi gửi `delivery_result` (`"outcome": "completed"`) gắn attempt/revision và sync hint.
- Khi nhận `reminder.failed` từ thiết bị, hoặc khi `SessionActor` phát hiện lượt phát `SpeechOrigin::Reminder` bị hủy giữa chừng (`playback_aborted`), hoặc khi ACK timeout / đóng WebSocket (`on_session_closed`) sau khi đã bắt đầu phát (`started_at_ms IS NOT NULL`), server chuyển attempt sang `failed`/`uncertain` kèm backoff `[5s, 15s, 60s]`; khi hết 3 lần thử hoặc quá cửa sổ trễ 24h (`LATENESS_GRACE_MS`) thì chuyển Reminder sang `expired` và tăng Schedule Revision.

Không cam kết exactly-once playback: mất mạng sau phát trước ACK có thể gây phát trùng; dedupe và bounded retry chỉ giảm rủi ro. Playback ACK chứng minh phần mềm drain sink, không chứng minh âm vật lý. Mỗi attempt mới là một TTS operation mới; từng ASR/LLM/TTS/MCP operation vẫn không automatic retry theo [ADR-0023](0023-zero-provider-retries.md).

Nguồn: [Guide §2, §4.3, §5.3, §9.4, §13–16](../voice_reminders_direct_tts_implementation_guide.md#53-reminder-audio-delivery--playback-ack-không-prepareready-v1).
