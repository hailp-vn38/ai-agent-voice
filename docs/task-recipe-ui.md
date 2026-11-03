# Giao diện Admin cho Task Recipe

Cập nhật: 2026-11-03. Trạng thái: đã triển khai trong Admin Web.

## Tạo Job

Trang Jobs tải danh mục bằng `GET /api/admin/background/recipes` khi mở modal **Tạo job**. Modal có ô tìm theo tên hoặc mô tả, thẻ **Tự do** và một thẻ cho mỗi recipe server trả về. Mỗi thẻ recipe hiện tên, mô tả và version. Lỗi tải danh mục hoặc tùy chọn động hiện trong modal.

Chọn recipe sẽ hiện form dựng từ `params_schema` và outline chỉ đọc. Form hỗ trợ:

| Schema | Control |
| --- | --- |
| `string` | Ô text; `format: uri` dùng ô URL |
| `integer` | Ô số nguyên, có `minimum`/`maximum` nếu schema khai báo |
| `boolean` | Checkbox |
| `enum` | Select; lựa chọn tĩnh lấy từ `enum`, tùy chọn động lấy theo `x-options-source` |
| `array<string>` | Textarea, mỗi dòng một mục |

Giá trị `default` được điền sẵn. Client chặn tham số thiếu hoặc nằm ngoài giới hạn số; server vẫn xác thực lại. Lỗi có `validation_path` được gắn cạnh trường tương ứng; lỗi còn lại hiện ở đầu form. Danh sách bước hiển thị số thứ tự, tiêu đề, loại Tool/Agent và nhãn lặp khi outline đánh dấu `repeat`.

Với recipe đầu tiên, danh sách domain Wiki động được nạp từ cây Wiki trong Admin API. Tùy chọn này không thay đổi nội dung recipe.

Recipe thu thập video yêu cầu đúng một trong hai đầu vào: URL playlist hoặc danh sách URL video, mỗi dòng một URL. Danh sách tối đa 50 mục; URL phải thuộc YouTube. Checkbox **Ingest vào Wiki sau khi thu thập** mặc định tắt. Khi tắt, video vẫn được lưu bền trong `sources/` nhưng Auto-Flow bỏ qua các nguồn đó cho đến khi người vận hành yêu cầu ingest rõ ràng; khi bật, nguồn đi vào Auto-Flow như luồng lưu hiện tại. Tiêu đề Job tự sinh tóm tắt loại đầu vào, giới hạn số video, domain cố định nếu có và lựa chọn Wiki; tiêu đề do người gọi nhập vẫn được giữ nguyên.

Chọn **Chạy recipe** gửi `recipe.id`, tham số và khóa idempotency. Khi tạo thành công, modal đóng và trang chi tiết Job mở. Thẻ **Tự do** giữ luồng tạo goal và tệp WAV/MP4 như trước; trường đính kèm chỉ hiện ở luồng này.

## Danh sách và chi tiết Job

Trong danh sách Jobs, Job recipe có dòng `recipe · <tên> v<version>` dưới goal.

Trang chi tiết hiển thị `RecipeProgress` phía trên timeline hiện có. Progress trình bày các bước thành thẻ trạng thái hoàn tất/đang chạy/chờ/thất bại, số mục đã xong trên tổng số ở bước lặp khi có dữ liệu, cùng `stop_reason` và bước lỗi. Đây là danh sách thẻ bước, không phải thanh tiến độ dạng phần trăm. Timeline giữ các Agent Run và tool call; nhãn lượt recipe hoặc lượt tự do hiện trên Agent Run khi server cung cấp `run_kind`.

## Phạm vi

Recipe chỉ được server đăng ký; giao diện không tạo, sửa hay xóa recipe. Không có trang danh mục riêng, preset tham số hoặc lập lịch recipe trong modal. Follow-up vẫn dùng giao diện Job hiện có.
