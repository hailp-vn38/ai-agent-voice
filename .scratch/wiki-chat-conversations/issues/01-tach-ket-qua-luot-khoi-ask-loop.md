# 01: Tách kết quả lượt ra khỏi Ask loop

**What to build:** Ask loop trả về một kết quả lượt có trạng thái rõ ràng thay vì tự phát sự kiện cuối và thoát ở nhiều nhánh. Sự kiện `done` được phát sau khi vòng lặp đã kết thúc. Admin không thấy thay đổi nào, nhưng từ đây có đúng một chỗ để các ticket sau ghi kết quả của lượt.

Kết quả lượt gồm: trạng thái (xong, bị hủy, lỗi kèm mã), phần câu trả lời đã có, các bước tool, trích dẫn và ảnh chụp đồ thị cuối.

Xem spec: [Vòng đời một lượt](../spec.md#vòng-đời-một-lượt).

**Blocked by:** None (can start immediately). Cần `feat/wiki-chat-graph` đã merge.

**Status:** resolved

- [x] Thứ tự và nội dung sự kiện SSE của `POST /knowledge/ask` không đổi với mọi kết thúc: xong, Dừng, ngắt kết nối, lỗi provider, `timeout`, `step_limit`
- [x] Lượt bị hủy vẫn trả về phần câu trả lời, bước tool và đồ thị đã có, kể cả khi hủy giữa lúc gọi tool hoặc dựng đồ thị
- [x] Ngắt kết nối vẫn dừng được provider giả
- [x] Toàn bộ test Wiki Chat hiện có qua mà không phải sửa kỳ vọng về hành vi


## Implementation result

Implemented and integrated on `feat/wiki-chat-conversations`. Focused server/web validation for this ticket passed through `scripts/agent-test.sh`; see the integration commits and test record in the implementation session.
