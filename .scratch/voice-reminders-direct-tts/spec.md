# Voice Reminders, Device Wake Scheduler & Direct TTS — V1

Status: ready-for-agent
Date: 2026-10-12
Source: implementation guide v1.1 và ADR-0087–0091.
Readiness: đặc tả sẵn sàng triển khai; chưa triển khai hoặc hardware-verified.

## Problem Statement

Người dùng chưa thể nhờ Voice Device lưu một lời nhắc rồi tự phát khi đến hạn. Hội thoại đang phụ thuộc vào Voice Session hiện tại; nếu ESP32 ngắt WebSocket, server không có cách gửi audio tới thiết bị và không thể chủ động đánh thức thiết bị đang Deep Sleep.

Admin cũng cần tạo, sửa, hủy và theo dõi lịch của từng Device, xem lịch tổng hợp theo Agent, thử phát text trực tiếp mà không qua nhận dạng lời nói hoặc mô hình ngôn ngữ. Người dùng cần biết chính xác lời nhắc đã được lưu, đã được thiết bị đồng bộ hay đã phát; gửi đủ audio qua socket không chứng minh loa đã phát hết.

## Solution

Cung cấp Reminder một lần, thuộc đúng một Device, được lưu bền vững trên server. Người dùng quản lý qua hội thoại giọng nói, Studio Assistant (Floating Chat) hoặc Admin Web. Đến hạn, server scheduler phát nội dung đã lưu bằng Direct Speech trên Voice Session của Device, không chạy LLM.

Firmware hỗ trợ lưu một cửa sổ lịch đánh thức, xác nhận sau khi lưu bền vững, dùng một logical wake timer rồi reconnect và xác thực lịch hiện tại trước khi phát. Device đang online dùng ngay WS đang có. UI phân biệt lưu server, đồng bộ cache, admission phát và Playback ACK; không hứa đúng giờ cho lịch chưa đồng bộ khi Device ngủ.

Cung cấp thêm client Direct TTS và Admin Direct TTS Test trên cùng pipeline audio. Các khả năng mới opt-in và deny by default, giữ tương thích firmware cũ.

## User Stories

1. As a Voice Device user, I want to say “remind me in ten minutes”, so that I can set a reminder without opening the Admin UI.
2. As a Voice Device user, I want relative time calculated by the server, so that a model-generated clock does not shift my reminder.
3. As a Voice Device user, I want ambiguous local times clarified, so that my reminder uses the intended timezone and date.
4. As a Voice Device user, I want to list my current reminders, so that I can check what is actually saved.
5. As a Voice Device user, I want to change a reminder's time or message, so that I can adjust my plans.
6. As a Voice Device user, I want to cancel a reminder, so that obsolete content is not played.
7. As a Voice Device user, I want confirmation only after saving succeeds, so that I do not rely on a reminder that was never stored.
8. As a Voice Device user, I want saved and synchronized confirmations distinguished, so that I know whether offline wake is available.
9. As a Voice Device user, I want due reminders spoken without another model call, so that delivery does not depend on model availability or rewriting.
10. As a Voice Device user, I want reminders to use my admitted TTS profile, so that delivery follows my Device's configured voice.
11. As a Voice Device user, I want a reminder played over the existing connection, so that I do not need a second socket.
12. As a Voice Device user, I want reminders to wait for a safe conversation boundary, so that they do not cut off my speech or the assistant's response.
13. As a Voice Device user, I want simultaneous reminders played sequentially, so that their audio does not overlap.
14. As a Voice Device user, I want abort and security invalidation to stop pending speech safely, so that cancelled audio does not continue.
15. As a Voice Device user, I want reminders preserved through server restarts, so that a process reboot does not erase my plans.
16. As a Voice Device user, I want reminders to stay with my Device when its Agent changes, so that reassignment does not lose or move them.
17. As an ESP32 user, I want synchronized reminders to wake and reconnect my Device, so that I can close its WS while idle.
18. As an ESP32 user, I want one wake timer for all reminders, so that many reminders do not require many timers or connections.
19. As an ESP32 user, I want wake-word detection preserved when required, so that reminder power management does not disable voice interaction.
20. As an ESP32 user, I want the Device to stay connected after waking early until the reminder is due, so that connection margin does not cause a missed reminder.
21. As an ESP32 user, I want cached reminders revalidated on reconnect, so that a reminder cancelled while I was offline is not played.
22. As an ESP32 user, I want the schedule cache to survive reboot or interrupted storage writes, so that a failed update does not corrupt the previous schedule.
23. As an ESP32 user, I want bounded reconnect backoff for overdue reminders, so that network failures do not create a wake loop or drain the battery.
24. As an ESP32 user, I want the cache replenished after a delivery batch, so that reminders beyond the first cache window are not forgotten.
25. As an ESP32 user, I want acknowledgement sent only after the audio sink drains, so that the server does not mark queued audio as played.
26. As an ESP32 user, I want terminal confirmation and an updated schedule before sleeping again, so that completed reminders do not cause another wake.
27. As an Admin, I want to create reminders for a specific physical Device, so that I can schedule a notification from the web.
28. As an Admin, I want to edit or cancel with conflict detection, so that concurrent changes do not silently overwrite each other.
29. As an Admin, I want a Device reminder list with sync and delivery status, so that I can diagnose why a reminder has not played.
30. As an Admin, I want a Reminders tab aggregating an Agent's current Devices, so that I can manage their schedules in one place.
31. As an Admin, I want filters, pagination and links to Device details, so that I can find a reminder in a large list.
32. As an Admin, I want live WS presence distinguished from Device enabled status, so that I do not mistake an allowed Device for a connected one.
33. As an Admin, I want offline-created reminders shown as awaiting sync, so that I do not promise wake before the firmware knows the schedule.
34. As an Admin, I want cache-window exclusions shown separately from acknowledged items, so that a global revision ACK does not misrepresent a specific reminder.
35. As an Admin, I want to try Direct TTS on an online Device, so that I can check its speech path without a conversation.
36. As an Admin, I want accepted requests distinguished from completed playback, so that a successful queue response does not mislead me.
37. As an Admin, I want to grant and revoke Reminder permission per Agent, so that only explicitly permitted conversations can manage schedules.
38. As a Studio Assistant user, I want to select a physical target Device through the Admin UI, so that reminders reach the intended hardware.
39. As a Studio Assistant user, I want missing or stale target selection rejected, so that the model cannot invent an authorized target.
40. As an authorized protocol client, I want to request text-to-speech directly, so that I can speak supplied text without ASR or LLM.
41. As an authorized protocol client, I want typed rejection for invalid or busy Direct TTS requests, so that I can handle failures without assuming speech started.
42. As a legacy firmware user, I want existing Voice and MCP Direct TTS behavior preserved, so that enabling new server capabilities does not require an immediate firmware upgrade.
43. As a system operator, I want offline waiting excluded from playback retry quota, so that a disconnected Device does not exhaust its reminders.
44. As a system operator, I want bounded persistent delivery attempts and expiry, so that uncertain delivery cannot retry forever.
45. As a system operator, I want repeated mutation requests replayed safely after restart, so that network retries do not create duplicate reminders.
46. As a system operator, I want stale session ACKs rejected, so that reconnect races cannot complete another attempt.
47. As a system operator, I want a single lifecycle-managed scheduler with restart recovery, so that reminders run in production and shut down safely.
48. As a system operator, I want privacy-safe metrics and correlated errors, so that I can diagnose late, failed or uncertain delivery without logging reminder content.
49. As a system operator, I want feature gates and a documented rollback procedure, so that I can roll out gradually without discarding persisted schedules.
50. As a maintainer, I want separate server and hardware acceptance evidence, so that a passing simulated playback test is not reported as verified offline wake.

## Implementation Decisions

### Domain ownership and persistence

- Theo ADR-0087, SQLite authoritative; Reminder có một Device owner, không thuộc Agent/Template. Query theo Agent join assignment hiện tại; đổi Agent không xóa/chuyển Reminder. Xóa Device dùng FK/retention nhất quán với chính sách hiện có và phải kiểm thử.
- Lifecycle Reminder chỉ gồm `pending`, `completed`, `cancelled`, `expired`. Delivery Attempt có identity, lease, session instance và persisted states `claimed`, `played`, `failed`, `uncertain`; streaming progress không là lifecycle bền vững. `waiting_for_device` là trạng thái suy ra.
- Module application-owned `DeviceNotifications` tập trung validate, caller authorization, CRUD, snapshot, claim, retry, ACK và audit. Admin HTTP, builtin tools và WS là adapters; không sao chép SQL/domain logic hoặc block SessionActor bằng database I/O.
- Forward-only migration bổ sung Agent Reminder permission default false, Reminder records, per-Device Schedule Revision/cache ACK, Delivery Attempts, mutation idempotency và cache membership. Chọn số migration từ baseline khi triển khai, không cố định số dự kiến trong guide.
- Mọi create/update/cancel/completed/expired thay đổi tập pending và tăng đúng một Schedule Revision của Device trong cùng write transaction; rollback/replay/attempt telemetry không tăng. Row revision CAS cho edit/cancel độc lập với Schedule Revision. Audit theo convention hiện hữu.
- Mutation idempotency dùng key unique theo authenticated caller namespace, request identity, operation và Device; payload hash/response ghi cùng mutation. Retry cùng payload replay response; khác payload trả conflict. Request identity tool call ổn định trong retry cùng call; call mới không mặc nhiên cùng hành động. Retention bounded không xóa key trước cửa sổ retry được hỗ trợ.
- Thời gian đến hạn do server tính tại transaction từ một trong `delay_seconds` hoặc `scheduled_at`; UTC epoch milliseconds và timezone diễn giải đầu vào. Relative duration V1 trong 1–31.536.000 giây, absolute time RFC3339 có offset hoặc timezone được xác định; thời gian mơ hồ cần hỏi lại. Quá hạn áp dụng lateness/expiry policy, không phát sớm.
- Text phải không rỗng sau trim, tối đa 1.024 Unicode characters và 4.096 UTF-8 bytes, kiểm soát control characters; validate tại trust boundaries, trước TTS admission.

### Production scheduler and delivery

- Khi enabled, production bootstrap start đúng một scheduler/process sau database/runtime init, recovery expired leases và scan due trước feature healthy. Scan bounded với tick nội bộ 1 giây, signal ngay sau mutation và reconnect; không task/sleep riêng cho mỗi Reminder hoặc scanner theo router clone.
- Shutdown đóng claim admission, cancel/drain/join bounded theo application lifecycle; leases chưa hoàn tất recover sau restart. Không giữ DB/registry lock qua await, không spawn unbounded delivery tasks.
- Application-owned Active Device Session registry map Device identity tới current instance, bounded mailbox, capabilities và presence. Register sau admission/ClientHello/actor init thành công; replace theo session replacement hiện hữu và unregister theo token khớp instance.
- Chỉ claim due khi có current opted-in capable session và capacity. Kiểm tra lại Device enabled, Reminder pending, quyền và session ngay trước claim/playback; hủy trước playback không được phát nội dung cũ. Offline/busy/enqueue failure trước start không tăng `attempts_used`; release/recover claim chưa start.
- `attempts_used` là tổng trên Reminder, tăng đúng một lần qua transaction fenced khi attempt thực sự bắt đầu phát; không reset qua reconnect/restart. Mỗi retry là attempt/TTS operation mới; không retry bên trong logical provider operation theo ADR-0023.
- Failed/aborted/ACK timeout hoặc mất WS sau khi start dẫn tới failed/uncertain với bounded exponential backoff. Hết quota hoặc lateness grace chuyển expired và tăng Schedule Revision. Playback ACK timeout dựa duration ước tính và sink latency budget với hard cap, cần calibration hardware.

### Direct Speech and protocol compatibility

- Theo ADR-0088, tổng quát MCP Direct TTS thành typed Direct Speech, phân biệt nguồn LLM/tool/client/Admin/Reminder. Server-originated request vào actor mailbox, không giả inbound client packet; tái sử dụng Active Turn Limiter, SpeechOutput, worker, resample/Opus/pacer, writer và Generation Gate.
- Direct Speech dùng TTS của Effective Session Profile đã admit, không chạy ASR/LLM, không commit user text/history/assistant message giả. Chỉ Speech Origin LLM phát message `type:llm`; direct text dùng TTS controls/source metadata cho client opt-in.
- Downlink giữ Opus 24 kHz mono 60 ms và writer start/audio/stop ordering. Start chỉ sau packet hợp lệ; provider failure trước first audio không gửi start/stop giả, sau start phải invalidate stale audio và terminalize đúng writer contract.
- Một active speech turn/Device. Reminder FIFO theo `(due_at, id)`, tối đa 16 queued items/actor: Processing/Speaking đợi turn, Listening có speech đợi utterance; idle Listening chỉ chuyển khi giữ đúng VAD/ASR state. Không cắt lời mặc định. Admin/client bận reject; requests ephemeral, không persist offline queue. Abort/security invalidation ưu tiên và trả failure cho caller phù hợp.
- Giữ Voice WS V1 và Device Admission. Optional features `reminders`/`direct_tts` mặc định false; feature không cấp quyền. Mọi application message mới sau hello bắt buộc khớp current `session_id`; không cho payload định tuyến sang Device khác. Client cũ không nhận Reminder controls hoặc metadata mới chưa opt-in.
- Client Direct TTS dùng `type:tts, action:speak`, request ID và text; không nhận client-assigned Device/provider/Agent/source. Cần feature opt-in, server/Agent grant `direct_tts.speak`, validation, rate và concurrency limits. `accepted` chỉ khi actor nhận/queue; typed reject gồm invalid text/session/request ID, unauthorized, unsupported, busy, capacity và TTS unavailable. Malformed/unknown legacy application messages vẫn theo ADR-0032.

### Durable cache synchronization and playback acknowledgement

- Theo ADR-0089, sau commit ảnh hưởng pending gửi `reminder.sync_required` revision hint tới active opted-in Device; coalesce giữ revision mới nhất. Nếu hint mất, reconnect luôn `sync_request` và scheduler due check ngay.
- Snapshot lấy từ read transaction nhất quán, tối đa 64 pending items sort `(due_at, id)`, mang snapshot ID, Schedule Revision, server time, `has_more`, `next_sync_at`. Firmware cache wake metadata, không cần full message; refill window sau batch.
- Device Cache ACK chỉ sau durable NVS commit và verify. Server validate authenticated Device, current session, snapshot token/revision và đúng membership đã gửi; ACK không lùi revision. Reminder chỉ `acked` khi có trong membership và snapshot không cũ hơn revision thay đổi của mục; >64 là `out_of_window`.
- `sync_status` độc lập delivery/lifecycle: `unknown`, `pending`, `acked`, `out_of_window`, `unsupported`. Saved/server commit, cache ACK, writer delivery và Playback ACK là bốn sự kiện riêng.
- Theo ADR-0090, Reminder audio start/stop bind Reminder và attempt identity. `tts:stop` là end-of-stream marker; firmware decode/play và drain toàn bộ output trước `reminder.played`. Decode/output/abort error báo bounded failure code, không giả played.
- Server chỉ completed sau ACK khớp Device/current session/Reminder/active attempt/lease. Duplicate ACK idempotent; old session ACK không complete attempt mới. Transaction đóng attempt, completed và Schedule Revision tăng, rồi gửi `delivery_result` đúng attempt/revision cùng sync hint.
- Firmware đợi đúng terminal result và ACK snapshot ít nhất terminal revision trước ngủ; mutation mới hơn vẫn cần sync current schedule. Mất WS/result/ACK dùng uncertain/reconnect policy; không coi socket bytes hoặc HTTP accepted là played. Playback ACK chỉ chứng minh phần mềm drain sink, không chứng minh âm vật lý.

### Firmware wake contract

- Implement trên firmware repository thật, ghi repo/commit/hardware profile khi bắt đầu; không tạo fake production firmware trong server repository. Native durable bounded versioned NVS blob ưu tiên; thêm two-slot journal chỉ khi power-cut evidence yêu cầu. Write failure giữ cache lành và retry/sync_failed.
- Một logical timer cho earliest future due trừ connection margin, next sync hoặc bounded retry deadline. Không arm past deadline/0ms loop; validate clock/overflow, minimum wait và dùng monotonic/RTC phù hợp lifetime/reset.
- Wake sớm, Wi-Fi/auth/Voice WS reconnect, luôn sync/revalidate rồi giữ connection tới UTC due/delivery hoặc bounded failure policy; không phát sớm và không sync xong rồi ngủ bỏ lỡ hạn. Nhiều due dùng một WS với audio tuần tự.
- Trước chủ động ngắt WS sau online mutation, firmware phải durable ACK current schedule hoặc áp dụng backoff an toàn. Sau terminal delivery, đợi confirmation, sync terminal/current revision và recompute future wake. Cache overdue chưa được server xử lý dùng bounded retry, không wake liên tục.
- Giữ wake word thì chỉ ngắt WS và dùng software timer; RTC Deep Sleep chỉ khi hardware/power policy cho phép. Schedule tạo khi ngủ chưa tới cache không bảo đảm đúng giờ. Cancel lúc ngủ có thể gây wake thừa nhưng reconnect không phát mục đã hủy.

### Builtin tools, authorization and prompt

- Theo ADR-0091, global `reminders.enabled` và persisted Agent `reminders_enabled` đều mặc định false. Admission chỉ quảng bá tools khi được cấp quyền; executor kiểm tra lại DB ngay trước mutation, revoke chặn stale session. Speaker Match, System MCP grants và MCP allowlist không tự cấp Reminder permission; `features.mcp=false` không chặn builtin có quyền.
- Bốn builtin tools: `server_create_reminder`, `server_list_reminders`, `server_update_reminder`, `server_cancel_reminder`. Create dùng text và đúng một relative/absolute time; update/cancel dùng ID + expected row revision, update có field patch; list bounded với status/limit và target chỉ trong scope cho phép. Reserve tên chống shadowing, async bounded execution/completion vào actor.
- Physical session mặc định và giới hạn authenticated Device hiện tại. Studio Assistant target là physical/enabled Device được Admin bind vào đúng active WS instance, server-side TTL ngắn; tool args/model/giọng nhắc tên Device không đổi target. Missing/stale/mismatch trả `target_selection_required`; Virtual Device không là RTC target.
- Quyền/target live guard độc lập immutable Effective Session Profile; delivery dùng admitted Agent/Template/TTS và quyền hiện thời, không hot reload profile/catalog.
- Tool round buffer lời khẳng định đến khi transaction thành công; trả `saved` cùng sync status thực, không hứa synced hoặc wake nếu chưa ACK.
- Compose một System message/turn với conditional reminder policy và escaped bounded reminders info (timezone, server time, bound Device/kind, capability); không full reminder text/list/pending count, không query list mỗi turn, không placeholder tùy tiện/rewrite stored Templates. Sanitize controls/XML delimiters, giữ cap 96 KiB và LLM request limit; tool continuation reuse snapshot cùng turn theo ADR-0084.

### Admin API, Web and operations

- Admin HTTP dùng middleware/auth/origin, request ID, error schema, conditional revision và rate limits hiện hữu. CRUD theo `/api/admin/devices/{device_id}/reminders`; aggregate current Agent Devices theo `/api/admin/agents/{key}/reminders`; Direct TTS Test dùng POST `/api/admin/devices/{device_id}/speak`; Studio target binding dùng PUT `/api/admin/system/assistant-device/reminder-target` với current session identity và selected physical Device.
- Device GET trả items, page và schedule summary gồm schedule/acked revision, ACK time, reported wake, actual session presence/capability; không tạo endpoint sync-status riêng. CRUD validation/not found/authorization/CAS/idempotency/unavailable trả typed errors theo conventions; timestamps thống nhất RFC3339/epoch fields. Admin speak trả 202 khi queued, offline/busy/unsupported trả typed failure.
- Agent Detail có Reminders tab, KPI dữ liệu thực, Device/status/time filters, paginated list và link Device; Device Detail có CRUD, sync/wake panel và Direct TTS Test. Studio Assistant có explicit target selector; Virtual Device không có physical wake controls. Reuse house components/API patterns, loading/error/confirmation, keyboard/ARIA và i18n vi/en.
- UI phân biệt saved, sync pending/acked/out of window/unsupported, waiting for Device, played/completed, uncertain, expired và actual WS presence. `enabled` là access setting, không là online. 202 chỉ hiển thị đã gửi yêu cầu, không đã phát.
- Operator interface tối thiểu: reminders enabled false; max pending/Device 500; retry limit 3 tổng attempts; lateness grace 86.400.000 ms; client inbound direct TTS enabled false. Validate limits; scan, text/cache/queue và lease bounds giữ constants hoặc existing runtime limits thay vì thêm knobs.
- Metrics theo create/due/complete/retry/expire/sync/ACK/lateness/Direct TTS result; không dùng Device/Reminder ID/message làm labels. Logs có attempt/session correlation nhưng không full reminder text mặc định. Có rollout, feature-disable behavior, bounded terminal retention, failure handling và rollback theo forward-only migration policy; không rollback bằng xóa lịch/schema.

## Testing Decisions

- Người dùng đã xác nhận ranh giới kiểm thử chính: Admin HTTP + Voice WebSocket; fake clock/TTS cho server, firmware/phần cứng thật cho offline wake. Ưu tiên một harness qua public boundary, không thêm test-only actor/SQL adapter cho từng caller. Assertions về trạng thái/response/audio/order quan sát được, không dựa private fields hay sơ đồ lớp.
- Tái sử dụng prior art hiện có: WS protocol integration với localhost listener và authenticated client, Admin API integration với HTTP client/provider doubles, Studio Assistant session tests, database bootstrap/migration tests, lifecycle readiness/shutdown tests, và public reference qualification với controlled process restarts. Web theo Vitest/Vue Test Utils và API fetch/auth/If-Match/error tests hiện có.
- DeviceNotifications dùng fake clock và SQLite fixture cho transactions, UTC/DST, CAS, idempotency concurrency/restart, revision rollback/replay và membership. Chỉ test seam thấp hơn khi khó quan sát race/clock ở public boundary; không kiểm tra hàm private hoặc mirror implementation.
- Migration gate: existing data preserved, Agent grant default deny, FK/delete behavior nhất quán, keys unique, forward-only boot; duplicate mutation sau restart chỉ một Reminder, payload mismatch conflict. Device reassignment đổi aggregate Agent nhưng không đổi owner.
- Sync gate: snapshot 0/1/64/65/500 items, stable order, has_more/refill; stale/unknown/wrong Device/session/token ACK không cập nhật cache hoặc lùi revision. Mọi create/update/cancel/complete/expire gửi hint sau commit; snapshot/ACK race vẫn eventual current sync.
- Scheduler gate qua production boot/restart: due được phát hiện cả khi chưa có WS; router clones không nhân scanner; expired leases recover; reconnect signal không chờ tick; offline một giờ/busy/enqueue fail trước start không tăng attempts; started retry không reset tổng; shutdown dừng claim và drain/cancel bounded.
- Direct Speech gate: authorized direct request có ASR=0, LLM=0, TTS=1; không stt/llm/history giả; đúng effective profile, valid start/audio/stop và canonical Opus. Invalid text/session/target/grant/request ID/rate/capacity không enqueue TTS; Admin accepted khác played.
- Actor/writer gate: Ready/Listening/Processing/Speaking, safe turn defer, queue full, template switch, abort/barge-in/security invalidation, backpressure và generation fencing; không interleave/drop text/audio. TTS lỗi trước first audio không start; lỗi sau start stop/cancel đúng. Session replacement không xóa handle mới hoặc nhận ACK cũ.
- Playback gate: năm Reminder cùng due dùng một connection, năm speech turns tuần tự và ACK sink drain riêng; cancelled-before-play không phát. Mất WS sau stop trước ACK là uncertain; duplicate ACK idempotent; current attempt valid mới completed, revision tăng và terminal result đúng identity; duration dài không timeout sớm.
- LLM/auth gate: “10 phút nữa nhắc tôi kiểm tra máy giặt” tạo một Reminder đúng server clock/Device, xác nhận sau commit; list query dữ liệu thực, update/cancel CAS. Agent revoke chặn mutation từ session cũ, MCP false vẫn dùng builtin khi có quyền; Floating Chat thiếu target deny, bound target đúng Device, stale/replaced session target không còn quyền.
- Prompt gate: disabled không lộ tools/policy, legacy Template vẫn render, escaped context không inject instruction, cap enforced và cùng-turn continuation giữ snapshot; không full list/pending count hoặc kế thừa target từ turn/session khác.
- Web gate: tab/filter/pagination/CRUD/routes đúng current Device/Agent, 409/auth/rate/validation errors, cancellation confirmation, explicit Studio target, accurate presence/sync/window/uncertain/expiry UI, accepted wording và accessibility/i18n.
- Hardware gate trên firmware thật: durable NVS ACK trước disconnect; reboot/power cut giữ schedule lành; một timer cho 100 reminders/window 64; reconnect trước due và giữ WS; năm same-time due một socket; sink drain ACK rồi delivery result rồi terminal sync ACK; overdue/clock skew/Wi-Fi mất mười phút không reconnect loop; cancel khi ngủ không phát; wake-word vẫn hoạt động với WS idle.
- Legacy matrix: firmware cũ, firmware mới, Studio Assistant và server reminders disabled; không controls/metadata mới khi không opt-in, không played giả từ socket delivery, Voice/MCP Direct TTS không regression.
- Khi triển khai chạy `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --workspace`; Web chạy scripts test, typecheck và build hiện có. Chạy lint nếu repo lúc triển khai có lint script; không giả định đã có. Ghi riêng firmware build/hardware evidence và các test NOT_RUN.

### Acceptance gates theo vertical slice

| Phase | Phạm vi | Điều kiện đạt |
|---|---|---|
| 0 | Baseline và canonical protocol fixtures | Baseline migrations/firmware commit xác định; new feature default off; golden controls/identity và legacy Voice/MCP regressions pass. |
| 1 | Direct Speech và Active Device Sessions | Public WS/Admin admission pass; ASR=0/LLM=0/TTS=1; writer/audio/source đúng; replacement không mất handle mới. |
| 2 | Persistence, live sync, scheduler bootstrap | Atomic revision/CAS/idempotency pass; online mutation có hint; startup/restart scan thật; offline không đốt retry; cloned router không thêm scanner. |
| 3 | Online delivery | Due không gọi LLM; sink-drain ACK mới completed; terminal result/revision/sync pass; cancellation/busy/uncertain/session races được chứng minh. |
| 4 | Firmware offline wake | Real hardware chứng minh NVS ACK → disconnect → one timer → authenticated reconnect/revalidate → delivery → drain ACK → terminal sync → future wake, có power-cut/network/window tests. |
| 5 | Builtin tools, prompt, authorized target | Voice create/list/update/cancel đúng scope; no premature confirmation; live revoke/Studio target guard/MCP independence/legacy prompts pass. |
| 6 | Web và vận hành toàn flow | CRUD/sync/presence/Direct TTS/target UI pass, Rust/Web suites pass, metrics/rollout docs và Phase 4 evidence đầy đủ. |

Build xanh không đủ đạt phase. UI/LLM có thể làm song song sau Phase 2 contract ổn định; không đồng thời đổi actor/protocol khi chưa chốt fixtures.

## Out of Scope

- Recurring/cron, một Reminder nhiều Device, LLM rewrite lúc due hoặc dùng LLM làm scheduler.
- Push đánh thức ESP32 Deep Sleep không có receive channel, cam kết đúng giờ cho lịch chưa cache ACK, offline TTS hoặc tự phát text cache chưa revalidate.
- Exactly-once physical playback hoặc suy đoán người dùng đã nghe lời nhắc.
- Notification WS, pipeline/runtime TTS thứ hai, MQTT broker mới hoặc FE giả Device WS.
- Deep Sleep mặc định làm mất wake word, mỗi Reminder một timer/task, prepare/ready handshake nếu chưa có hardware evidence.
- Full reminder list trong prompt, placeholder/template engine mới, global state store hoặc nhiều config knobs không cần thiết.
- Mock firmware thay thế production hardware acceptance; production stubs/simulated success.
- Push, merge hoặc triển khai tính năng trong công việc tạo spec này.

## Further Notes

- Nguồn chi tiết: [implementation guide v1.1](../../docs/voice_reminders_direct_tts_implementation_guide.md); vocabulary: [GLOSSARY](../../GLOSSARY.md#reminders-and-direct-speech) và [CONTEXT](../../CONTEXT.md).
- ADR thiết kế: [0087 — ownership/persistence/scheduler](../../docs/adr/0087-device-owned-durable-reminders.md), [0088 — Direct Speech](../../docs/adr/0088-shared-session-direct-speech.md), [0089 — sync/wake](../../docs/adr/0089-reminder-snapshot-sync-and-device-wake.md), [0090 — playback/attempts](../../docs/adr/0090-reminder-playback-ack-and-bounded-attempts.md), [0091 — authorization/prompt](../../docs/adr/0091-reminder-authorization-and-turn-context.md). Các ADR đang Proposed; spec ghi mục tiêu triển khai, không tuyên bố Accepted/implemented.
- Giữ các quyết định nền về SessionActor, single writer, playable-first start, writer terminal outcome, immutable profile và forward-only migration. Phạm vi mở rộng ADR-0023/0032/0052/0084 đã được ghi riêng: attempt retry khác provider retry, typed opt-in outcomes khác legacy faults, live authorization khác hot reload, turn context khác template rewrite.
- Không có firmware ESP32 hoàn chỉnh trong server repository. Repo/commit firmware, hardware/power profile, measured connect margin, sink drain signal và storage recovery cần ghi trước Phase 4; TTL/retention/ACK cap phải được chốt và validate theo integration policy khi triển khai, không bỏ qua giới hạn.
- Full DoD cần đủ online Voice/Admin creation → save/sync → due Direct Speech → drain ACK → completed/terminal sync và offline hardware wake/reconnect, multiple reminders, cancellation while asleep, offline-created sync pending, legacy compatibility, restart/idempotency/session-fence recovery.
- Nếu chỉ server/Web/harness pass mà thiếu hardware, bàn giao chỉ `server-ready, firmware-unverified`; lịch chưa ACK không bảo đảm offline wake và mất ACK sau playback có thể phát trùng khi retry.
- Bàn giao implementation phải có migration/change inventory, official protocol payloads/flows, compatibility matrix, actual Rust/Web/firmware test results, hardware logs, rollout/grant/retention/late-failure handling và rollback instructions. Không ghi PASS cho test chưa chạy.
- Spec này được publish vào issue tracker local với `ready-for-agent`; chưa chia implementation tickets. Trạng thái này không xác nhận tính năng đã nghiệm thu.
