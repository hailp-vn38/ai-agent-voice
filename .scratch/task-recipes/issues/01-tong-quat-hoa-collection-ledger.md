# 01: Tổng quát hóa Collection Ledger theo loại mục

**What to build:** Collection Ledger ghi được mục thuộc nhiều loại, không chỉ video của playlist. Đây là prefactor để bước lặp theo lô của Task Recipe dùng lại cùng sổ theo dõi. Với người dùng, hành vi thu thập playlist và Wiki Auto-Flow không đổi.

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] Mỗi mục trong ledger có loại mục và khóa định danh theo loại. Video playlist hiện có được migrate sang loại riêng bằng migration forward-only.
- [ ] Không trùng, không ghi đè, làm tiếp sau restart và cách ly nguồn lỗi vẫn đúng như ADR 0136.
- [ ] Toàn bộ integration test hiện có về thu thập playlist và Auto-Flow pass mà không phải sửa kỳ vọng.
- [ ] Có test qua interface TaskManager cho thấy một loại mục thứ hai được ghi và đọc lại độc lập với video playlist.
