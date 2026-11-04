# Spike độ bền session Muse

Trả lời chín câu hỏi ở mục [Spike bắt buộc](../agent-job-durability-implementation-guide.md#spike-bắt-buộc) bằng tài khoản Muse thật. **Chưa chạy lần nào.** Lớp đọc lại session đã có test với VM giả (`spike_probe_reads_history_and_file_without_the_original_stream`), nhưng hành vi của Muse thật chưa được đo.

Mã nguồn: `crates/vela-server/src/providers/llm/gateway/muse/workflow/media/live_spike.rs`.

## Chi phí và an toàn

- Tiêu quota video thật: mức `abandon` tạo một video 8s, mức `clips` thêm khoảng 80s video.
- Test luôn `#[ignore]` và dừng ngay nếu thiếu `MUSE_SPIKE_CONFIRM=spend-quota`.
- Test dọn session của job khi kết thúc. Nếu `session_deleted` là `false`, hoặc test dừng giữa chừng, phải xóa tay session trong `session_id`.
- Báo cáo JSON không chứa credential. Không commit file credential.

## Chạy

Giá trị credential là chuỗi đang dán vào Admin khi thêm tài khoản Muse (`parse_accounts` nhận cookie hoặc mảng JSON). Dùng tài khoản thứ hai cho câu hỏi 2; bỏ qua được nếu chỉ có một tài khoản.

```bash
read -rs MUSE_SPIKE_CREDENTIAL_A; export MUSE_SPIKE_CREDENTIAL_A
read -rs MUSE_SPIKE_CREDENTIAL_B; export MUSE_SPIKE_CREDENTIAL_B
export MUSE_SPIKE_CONFIRM=spend-quota
export MUSE_SPIKE_OUTPUT="$PWD/muse-spike-abandon.json"

# Khoảng 16 phút. Ghi quota trước và sau lần chạy (câu hỏi 5).
RUST_LOG=info scripts/agent-test.sh run -- cargo test --quiet -p vela-server --lib \
  muse_durability_live_spike -- --ignored --nocapture

# Câu hỏi 6: đợi hết 900s để giữ log của lượt thất bại nếu có; đặt mốc đọc ngắn.
MUSE_SPIKE_ABANDON_AFTER_SECS=900 MUSE_SPIKE_CHECKPOINT_MINUTES=1 \
MUSE_SPIKE_OUTPUT="$PWD/muse-spike-900s.json" \
RUST_LOG=info scripts/agent-test.sh run -- cargo test --quiet -p vela-server --lib \
  muse_durability_live_spike -- --ignored --nocapture

# Câu hỏi 7 và 8: tạo video dài, lưu MP4 để so bằng mắt.
mkdir -p muse-spike-media
MUSE_SPIKE_STAGES=clips MUSE_SPIKE_MEDIA_DIR="$PWD/muse-spike-media" \
MUSE_SPIKE_OUTPUT="$PWD/muse-spike-clips.json" \
RUST_LOG=info scripts/agent-test.sh run -- cargo test --quiet -p vela-server --lib \
  muse_durability_live_spike -- --ignored --nocapture
```

| Biến | Mặc định | Ý nghĩa |
| --- | --- | --- |
| `MUSE_SPIKE_STAGES` | `abandon` | `abandon`, `clips`, hoặc cả hai |
| `MUSE_SPIKE_ABANDON_AFTER_SECS` | 45 | Bỏ ngang lượt gửi sau số giây này |
| `MUSE_SPIKE_CHECKPOINT_MINUTES` | `1,5,15` | Các mốc đọc lại sau khi đứt kết nối |
| `MUSE_SPIKE_OUTPUT` | `muse-durability-spike-result.json` | Báo cáo JSON |
| `MUSE_SPIKE_MEDIA_DIR` | Không lưu | Thư mục lưu MP4 của mức `clips` |

`agent-test.sh` chỉ in phần tóm tắt. Lấy kết quả từ file JSON và log đã lưu.

## Đọc kết quả

Ghi kết quả vào `docs/providers/muse-durability-spike-results-<ngày>.md`, rồi quyết định theo cột cuối.

| # | Câu hỏi | Xem ở | Đạt khi | Nếu không đạt |
| --- | --- | --- | --- | --- |
| 1 | Session sống sau khi đứt WebSocket? | `checkpoints[].owner` | `history_status=200` và `user_turns>=1` ở mọi mốc | Dùng `VIDEO_SESSION_LOST`, chỉ giữ được trong một process |
| 2 | Có cần cùng account? | `checkpoints[].other_account` | Ghi nhận được là có hay không | Có cần: giữ khóa `account_key`, không failover (code đang làm thế) |
| 3 | `session_id` client tạo có được giữ? | `session_id`, `owner.user_turns` | Đọc history bằng chính id đó thấy lượt user | Không đối soát được `submitting` |
| 4 | History đủ `presentation` khi không có stream? | `owner.ready_paths` | Có đường dẫn khi video đã xong | Phải đọc thêm từ event |
| 5 | Bỏ ngang có còn tính quota? | So quota trên tài khoản, bằng tay | Ghi nhận được là có hay không | Có tính: giảm retry video |
| 6 | Vì sao 900s thất bại? | `abandoned_turn` ở lần 900s và log `info` | Có lý do cụ thể | Sửa theo lý do, có thể không cần chờ giai đoạn 4 |
| 7 | 6 clip tính theo lượt hay session? | `clip_limit_turns[1]` | Lượt 20s thành công với đủ thời lượng là theo lượt | Mỗi lô mở session riêng và gửi lại đủ kịch bản |
| 8 | Muse có dùng lại clip cũ khi được nhắc? | MP4 trong `MUSE_SPIKE_MEDIA_DIR`, so bằng mắt | Video lượt sau chứa lại các cảnh đã có | Bỏ vòng sửa, tạo lại cả video |
| 9 | File còn đọc được sau khi đứt? | `owner.read_bytes`, `owner.read_error` | Có `read_bytes` ở mọi mốc sau khi có file | Lưu file từng lô thành artifact trước khi gửi lô sau |

Câu hỏi 8 chỉ được kiểm tra đúng nhánh `MISSING_DURATION` khi lượt đầu trả thiếu thời lượng. Nếu Muse trả đủ ngay, ghi là chưa kích hoạt được thay vì coi là đạt. `read_bytes` tối đa 256 KiB vì chỉ cần chứng minh đọc được.

Kịch bản kill thật ở giai đoạn 4 (`Xong khi`) là bước riêng, chạy trên server đầy đủ sau khi có kết quả này.
