# 05: Giới hạn lưu trữ

**What to build:** Lịch sử Wiki Chat không thể làm DB lớn lên vô hạn. Khi vượt giới hạn, hệ thống xử lý theo quy tắc đã định và Admin vẫn dùng được: hội thoại cũ nhất tự bị xóa, hội thoại đầy thì được gợi ý mở cái mới, câu trả lời dài bị cắt có đánh dấu, đồ thị quá lớn không được lưu.

Xem spec: [Giới hạn](../spec.md#giới-hạn).

**Blocked by:** 02

**Status:** resolved

- [x] Tạo hội thoại thứ 201 xóa hội thoại có `updated_at_ms` cũ nhất, bỏ qua hội thoại đang chạy
- [x] Hội thoại đã có 100 tin nhận `409 wiki_conversation_full` và không có dòng nào được ghi; web gợi ý mở hội thoại mới
- [x] Câu trả lời trên 64 KiB được lưu đến đúng ranh giới ký tự UTF-8, đặt `content_truncated`, trạng thái vẫn đúng và trích dẫn vẫn đủ; web cho biết câu trả lời đã bị cắt
- [x] `tools_json` trên 64 KiB bỏ các bước cuối, không làm lượt thất bại
- [x] `graph_json` trên 256 KiB được lưu `NULL`, lượt vẫn `done`, web ẩn nút xem đồ thị
- [x] Không có setting và không có xóa theo thời gian


## Implementation result

Implemented and integrated on `feat/wiki-chat-conversations`. Focused server/web validation for this ticket passed through `scripts/agent-test.sh`; see the integration commits and test record in the implementation session.
