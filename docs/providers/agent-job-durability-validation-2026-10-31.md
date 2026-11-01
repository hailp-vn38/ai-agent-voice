# Kiểm chứng bổ sung độ bền Agent và Job

Đợt triển khai tiếp nối review ngày 2026-10-31; kiểm chứng tiếp tục sang 2026-11-01. Spec: [implementation guide](../agent-job-durability-implementation-guide.md), [thiết kế](../agent-job-durability-redesign.md).

Phạm vi so với đầu phiên:

- Repo chính: `7be4c6a0`, nhánh `feat/muse-noise-websocket-provider`.
- Agent vendor: `e1a73b6`, nhánh `feat/job-skill-web-management`; bao gồm phần ngân sách đang chưa commit ở đầu phiên.
- Task vendor: `faa6d7e`, nhánh `feat/video-continuation-key`; bao gồm phần ánh xạ ngân sách đang chưa commit ở đầu phiên.

## Standards

Review độc lập phát hiện một xung đột bắt buộc: glossary/ADR đặt ngân sách trên cả vòng đời Task, trong khi follow-up nhận ngân sách mới. Đã sửa GLOSSARY và ADR 0133: ngân sách theo turn, giữ qua retry/restart, cấp mới cho Task Follow-up.

Hai nhận xét thiết kế:

- Đã thay kiểm chuỗi `pending_turn` bằng kiểm trường JSON.
- Còn SQL cập nhật hết ngân sách bị lặp trong `store/recovery.rs`. Đây là nhận xét tùy chọn, chưa refactor trong đợt sửa hành vi.

Đã gom policy live recovery vào `RecoveryContext` cho scheduled, user và operator resume. Startup giữ routing tĩnh từ cấu hình ban đầu. Review lại: **không còn finding Standards bắt buộc**.

## Spec

Review phát hiện bốn lỗi hành vi và một trường hợp tương thích dữ liệu cũ; đã sửa cả năm:

| Phát hiện | Sửa và bằng chứng |
| --- | --- |
| Attach follow-up bỏ qua journal | Mọi lượt nhắc đính kèm lưu pending turn và cờ một lần xuyên restart; `attachment_followup_is_journaled_before_sending` |
| POST được nhận nhưng mất ACK có thể failover | Durable transport/ACK timeout trả outcome unknown, giữ session; `lost_submission_ack_keeps_session_for_reconciliation_without_failover` |
| Gửi lô/repair sau deadline hoặc báo thành công với video thiếu | Kiểm deadline trước mỗi POST và sau các lần đọc; collect hết giờ trả timeout; `deadline_expiring_during_download_prevents_next_batch_submission`, `repair_deadline_does_not_report_partial_video_as_success` |
| Registry chỉ giữ SID cuối của job | Dọn từng `(account_key, remote_session_id)` và chỉ đánh dấu sau ACK; `cleanup_deletes_all_remote_sessions_of_one_job` |
| Checkpoint cũ `running` giữa lô không có pending turn | Trả `TOOL_OUTCOME_UNKNOWN` để đối soát; `legacy_between_batch_checkpoint_requires_reconciliation` |

Review Spec cuối xác nhận không còn blocker trong các phát hiện đã review; các gate live bên dưới vẫn độc lập. Các test đã được chạy đỏ trước sửa cho shutdown mất invocation, grace ngắn, session bị coi là terminal, resume giữa lô, deadline sau download, timeout repair, nudge không journal, nhiều SID và checkpoint legacy. Có lượt compile test thất bại do fixture/import mới; đã sửa và chạy lại. Các lần thất bại giữ nguyên mã thoát và log theo wrapper, không được coi là pass.

Các bằng chứng khác:

- `planned_shutdown_resumes_video_create_without_cancelling_operation`: qua `Orchestrator::shutdown`, tool `video_create` thật, backend hợp tác hủy, khởi tạo orchestrator mới và commit artifact sau resume.
- `shutdown_preserves_resumable_invocation_for_restart`: giữ nguyên invocation ID qua shutdown/restart.
- `cancel_video_within_grace`: kiểm mốc 30–35s theo đồng hồ Tokio ảo, không chỉ đo thời gian thật.
- `resumable_panic_with_open_operation_does_not_settle_as_failure`: không cho model tiếp tục tạo invocation mới khi operation còn mở.
- `duration_repairs_are_journaled_per_batch`, `checkpoint_failure_prevents_remote_submission`: counter theo lô và không POST khi không ghi được checkpoint.
- `task_waiting_cleared_when_orchestrator_resumes`: TaskManager đọc lại run chạy tiếp, xóa lịch/lý do chờ mà không cần gọi API resume của TaskManager.
- `random_fault_job_stops_at_budget_limit` và `task_budget_exhausted_becomes_stopped`: ngân sách dừng có mã rõ ràng và ánh xạ Task sang `Stopped`.
- UI tests kiểm trạng thái chờ, output từng phần, ngưỡng cảnh báo 5 phút, bộ đếm retry theo cấu hình, footer không báo hoàn tất khi còn chờ, và đồng hồ dừng tại finished_at cho job Stopped; không ghi cứng mẫu số 10.

## Lệnh kiểm chứng cuối

Mọi lệnh test/check đều chạy qua `scripts/agent-test.sh`; không chạy `cargo test` trực tiếp.

| Lệnh | Kết quả |
| --- | --- |
| `scripts/agent-test.sh check` | PASS trên cây thật |
| `scripts/agent-test.sh full` | PASS trên cây cuối |
| `scripts/agent-test.sh run -- cargo test --quiet --manifest-path vendor/orchestrator_agent_rs/Cargo.toml` | PASS |
| `scripts/agent-test.sh run -- cargo test --quiet --manifest-path vendor/orchestrator_task_rs/Cargo.toml` | PASS |
| `scripts/agent-test.sh web test` | PASS: 83 files, 403 tests |
| `scripts/agent-test.sh web typecheck` | PASS |

Targeted tests được chạy trước full suite. Có một lượt full workspace dừng ở hai fixture `TaskDetailView` trong `background_http.rs` thiếu trường giới hạn retry mới; đã cập nhật fixture trước lượt cuối. Một lượt full task còn phát hiện race của `run.started` chốt terminal trước khi tiêu hết message event; watcher đã đổi sang chỉ cập nhật trạng thái chờ ở nhánh này và full task sau đó pass. Test FileReader trên web cũng đã thay chờ cứng 25ms bằng chờ điều kiện đọc xong; full web sau đó pass; lượt cuối gồm 403/403 test. Một lượt Muse suite được dừng bằng SIGTERM khi test timeout cũ dùng thời gian thật sau khi deadline được sửa thành profile dài hơn; test đó đã chuyển sang đồng hồ Tokio ảo và nhóm Muse sau đó pass.

Format chỉ áp dụng các file Rust trong phạm vi sửa. Các thay đổi chỉ định dạng do `cargo fmt --all` gây ra ngoài phạm vi đã được hoàn tác. Không đưa tài liệu Chromium/memory, `node_modules/` hoặc thay đổi không liên quan vào commit.

## Migration trên bản sao DB thật

Tạo bản sao nhất quán bằng SQLite backup API, rồi chạy SQLx Migrator với cùng migration sources và policy khóa ngoại của production: connection riêng `foreign_keys(false)` cho agent; khóa ngoại bật cho task. Không chạy migration trên DB gốc.

| DB nguồn | Version trước → sau trên bản sao | foreign_key_check | integrity_check |
| --- | --- | --- | --- |
| `data/orchestrator-store/orchestrator.db` | 7 → 8 | 0 lỗi | ok |
| `data/orchestrator-store/wiki/orchestrator.db` | 4 → 8 | 0 lỗi | ok |
| `data/orchestrator-store/jobs/orchestrator.db` | 4 → 8 | 0 lỗi | ok |
| `data/tasks-store/tasks.db` | 2 → 2 | 0 lỗi | ok |

Lệnh được bọc bằng `scripts/agent-test.sh run -- cargo run ...`; entrypoint kiểm chứng tạm đã xóa khỏi cây source. Bản sao và manifest/report tạm lưu ngoài repo, không đưa nội dung DB hoặc credential vào tài liệu/commit.

## Gate chưa chạy và giới hạn phát hành

**Giai đoạn 4 chưa được xác nhận phát hành.** Các mục dưới đây không được suy ra từ test giả:

| Câu hỏi spike Muse | Trạng thái |
| --- | --- |
| Session sống sau ngắt WebSocket 1/5/15 phút | NOT_RUN |
| Cùng account so với account khác | NOT_RUN |
| Client session ID so với assigned ID thật | NOT_RUN |
| History giữ đủ presentation khi không có stream | NOT_RUN |
| Quota trước/sau bỏ ngang | NOT_RUN |
| Nguyên nhân lỗi video thật ở 900s | NOT_RUN |
| Giới hạn 6 clip theo turn hay theo session | NOT_RUN |
| Reuse clip qua MISSING_DURATION thật | NOT_RUN |
| Đọc file sau hoàn tất/ngắt kết nối 1/5/15 phút | NOT_RUN |

Kịch bản SIGKILL giữa render/lô/download qua Muse thật và xác nhận số session thật còn lại bằng 0: **NOT_RUN**. Không kết luận spike thất bại; chưa có căn cứ chọn fallback `VIDEO_SESSION_LOST`.

Hai nhánh vendor chưa merge vào `main`; không push hoặc triển khai server trong đợt này. Cấu hình tạm giai đoạn 0 chưa được xác nhận qua Admin/log; code Muse chat đã dùng timeout profile và lưới an toàn `max_total + 30s`.

File runtime `data/orchestrator-prompts/media_creator.md` tồn tại và có hướng dẫn `max_total_duration_secs`/`video_concat`, nhưng khác seed hiện tại và thiếu hướng dẫn `same job`. Cần đồng bộ có giữ tùy chỉnh và xác nhận reload trước phát hành; đợt này không ghi đè prompt runtime.

## Commit triển khai

Các gate kiểm chứng code nêu trên đã đạt; các gate Muse thật vẫn chưa chạy.

- Agent vendor: `a88bbd1` — persist run budgets and preserve resumable recovery.
- Task vendor: `533be88` — enforce turn budgets and preserve resumable video work.

Repo chính lưu guide, thiết kế và biên bản; thư mục vendor vốn được `.gitignore` và có Git repository riêng. Commit trên các nhánh hiện tại, không merge `main` hoặc push.

## Đợt bổ sung ngày 2026-11-01

Phạm vi: các sửa review ghi trong [guide](../agent-job-durability-implementation-guide.md#sửa-sau-code-review-ngày-2026-11-01) và phần còn thiếu sau đó. Trên vendor agent, đợt này nằm ở [patch](patches/orchestrator_agent_rs-durability-remaining.patch), áp lên working tree đang chứa các sửa review (nền `a88bbd1`).

### Nguyên nhân và cách sửa ba lỗi kiểm thử

| Test | Nguyên nhân gốc | Sửa | Trước | Sau |
| --- | --- | --- | --- | --- |
| `shutdown_waits_for_cooperative_tool_and_commits_its_result` | Lỗi code: `shutdown_grace` 5s làm ân hạn tool về 0, tool hợp tác bị drop, sau restart bị gọi lại và treo. Cấu hình mặc định 30s không bị. Biên bản trước ghi suite này pass, nên lỗi không ổn định theo lần chạy | `shutdown_cancel_grace` giữ tối thiểu một nửa `shutdown_grace` | 13/15 fail trên bản gốc | 0/60 |
| `at29_st04_child_events_reach_the_family_before_the_parent_tool_completes` | Lỗi code: run con commit trạng thái kết thúc trước khi phát event, nên cha đọc store rồi phát `tool.completed` trước `run.completed` của con | `await_child` chờ tối đa 500ms cho event kết thúc của con, chỉ khi đã thấy con đang chạy và chưa nhận event | 2/60 fail, CPU tải cao | 0/150, cùng tải |
| `budget_capped_text_only` | Lỗi test: `now_secs` trộn đồng hồ thật và đồng hồ tokio đã pause, có thể lệch 1s; tool ngủ đúng bằng hạn mức 100s nên `active_secs` có thể là 99 | Tool ngủ 102s. Sai số 1s không đáng kể với ngân sách thật tính bằng giờ | 2/200 fail | 0/300 |

### Kết quả

| Kiểm tra | Kết quả |
| --- | --- |
| Full suite vendor agent, `--no-fail-fast` | PASS, trước lần `rustfmt` cuối của patch |
| Full suite vendor task, trên agent đã áp patch | PASS |
| Toàn workspace, `--no-fail-fast`, có `config.toml` local | PASS |
| `cargo clippy -D warnings`: workspace | PASS |
| `cargo clippy -D warnings`: vendor agent | PASS, trước lần `rustfmt` cuối của patch |
| `cargo fmt --all --check` | Sạch |
| `terminal_release_waits_for_the_active_run_then_deletes_the_session_once` | PASS |
| `spike_probe_reads_history_and_file_without_the_original_stream` | PASS |
| `scripts/sync-media-creator-prompt.py` trên bản sao `agent_settings.json` | Thay đúng mục video, giữ các mục khác; chạy lại không đổi. Chưa áp vào dữ liệu thật |

`vieneu` đọc `config.toml` ở gốc repo, nên `full` fail trong worktree mới không có file này. Đó là phụ thuộc môi trường có sẵn, đợt này không đổi.

Vendor task không có thay đổi trong đợt này. Không chạy lại test web vì không sửa web.

### Chưa chạy

Chín phép đo Muse thật ([runbook](muse-durability-spike.md)), kill thật và xác nhận số session Muse thật còn lại bằng 0 vẫn **NOT_RUN**. Giai đoạn 4 chưa đủ điều kiện phát hành.
