# 03: Kiểm chứng độ bền của Task recipe

**What to build:** Chứng minh rằng Task recipe bền như Task thường. Phần lớn hành vi đã có sẵn từ runtime vì recipe là Task thường với một Root đặc biệt, nên ticket này chủ yếu là test. Code chỉ được sửa ở những chỗ test chỉ ra lỗ hổng.

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] Restart khi Tool Step `ReadOnly` đang chạy thì Task resume và chạy tiếp tới hoàn tất, không lưu trùng nguồn.
- [ ] Tool Step `Mutating` bị gián đoạn khi restart thì Task chặn ở đường gỡ tool kẹt hiện có, không chạy lại.
- [ ] Agent Step con gặp 429 thì Task vào Waiting Retry rồi chạy tiếp, không làm lại Tool Step đã xong.
- [ ] Recipe dùng chung Job Budget. Hết ngân sách thì Task `stopped`, giữ kết quả từng phần. Lượt recipe không trừ token.
- [ ] Hủy Task giữa một lô thì các tool call đang chạy dừng trong thời gian ân hạn, như Task thường.
- [ ] Resume với version đã ghim khác bản trong registry thì Task kết thúc với `RECIPE_VERSION_UNAVAILABLE`.
- [ ] Playlist dài nhất cho phép (`max_steps` và số video tối đa) chạy hết mà không chạm step cap, và compaction không làm `next` đọc sai tiến độ.
- [ ] Mỗi hành vi trên có integration test qua interface TaskManager.
