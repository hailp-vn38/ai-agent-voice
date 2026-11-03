# 03: Kiểm chứng độ bền của Task recipe

**What to build:** Chứng minh rằng Task recipe bền như Task thường. Phần lớn hành vi đã có sẵn từ runtime vì recipe là Task thường với một Root đặc biệt, nên ticket này chủ yếu là test. Code chỉ được sửa ở những chỗ test chỉ ra lỗ hổng.

**Blocked by:** 02

**Status:** resolved

- [x] Restart khi Tool Step `ReadOnly` đang chạy thì Task resume và chạy tiếp tới hoàn tất, không lưu trùng nguồn.
- [x] Tool Step `Mutating` bị gián đoạn khi restart thì Task chặn ở đường gỡ tool kẹt hiện có, không chạy lại.
- [x] Agent Step con gặp 429 thì Task vào Waiting Retry rồi chạy tiếp, không làm lại Tool Step đã xong.
- [x] Recipe dùng chung Job Budget. Hết ngân sách thì Task `stopped`, giữ kết quả từng phần. Lượt recipe không trừ token.
- [x] Hủy Task giữa một lô thì các tool call đang chạy dừng trong thời gian ân hạn, như Task thường.
- [x] Resume với version đã ghim khác bản trong registry thì Task kết thúc với `RECIPE_VERSION_UNAVAILABLE`.
- [x] Playlist dài nhất cho phép (`max_steps` và số video tối đa) chạy hết mà không chạm step cap, và compaction không làm `next` đọc sai tiến độ.
- [x] Mỗi hành vi trên có integration test qua interface TaskManager.

## Answer

Đã merge vào integration branch `feat/task-recipes` trong flow `b031066`; TaskManager compaction regression phát hiện và sửa RecipeProviderRuntime không đọc lại params từ `<original_request>` sau khi summary. Integration tests chứng minh restart ReadOnly, không replay Mutating, retry 429 không làm lại Tool Step đã xong, Job Budget giữ partial result với usage 0, cancellation, version thiếu, playlist tối đa 50 và compaction runtime chọn batch từ ledger. Flow `scripts/check.sh test` và targeted server/flow tests pass. `scripts/check.sh fast` vẫn dừng ở clippy cảnh báo nền `src/tools/pdf.rs:142` (`too_many_arguments`), ngoài diff.
