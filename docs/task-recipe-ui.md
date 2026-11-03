# Giao diện tạo Job từ Task Recipe

Cập nhật: 2026-11-03. Trạng thái: đề xuất, đi kèm [ADR 0140](adr/0140-server-owned-task-recipes.md).

Modal "Tạo job" ở `apps/admin-web/src/pages/jobs/JobsPage.vue` hiện chỉ có ô mục tiêu và đính kèm WAV. Đề xuất: modal này mở bằng danh sách recipe. Người dùng chọn một recipe, điền tham số theo form dựng từ schema, xem trước các bước rồi bấm chạy. Ô "Tự do" vẫn là lựa chọn đầu tiên và giữ nguyên hành vi hiện tại.

## Luồng

1. Bấm **Tạo job** trên trang Jobs.
2. Modal hiện ô tìm kiếm và lưới thẻ: thẻ **Tự do** đứng đầu, tiếp theo là các recipe đang bật. Mỗi thẻ có tên, mô tả ngắn và version.
3. Chọn recipe thì phần dưới hiện form tham số và danh sách bước chỉ đọc, ghi rõ bước nào là Tool, bước nào là Agent và bước nào lặp theo lô.
4. Bấm **Chạy job**. Client kiểm tra trường bắt buộc; server kiểm tra lại và là nơi quyết định cuối cùng.
5. Tạo xong thì đóng modal và mở trang chi tiết Job, như luồng hiện tại.

Không có trang danh mục recipe riêng ở v1. Modal là nơi duy nhất để xem và chạy recipe, vì recipe nằm trong code và Admin không sửa được nội dung của nó.

## Form từ schema tham số

Server trả schema tham số theo một tập con nhỏ, cùng tinh thần với ADR 0059. Client chỉ render những kiểu dưới đây; recipe dùng kiểu khác thì server không được đăng ký nó.

| Kiểu schema | Control | Ghi chú |
| --- | --- | --- |
| `string` | `input` | `format: uri` thành ô URL |
| `integer` | `input type=number` | Dùng `minimum` và `maximum` |
| `boolean` | checkbox | |
| `enum` | `select` | Có `x-options-source` để lấy danh sách động, ví dụ domain Wiki |
| `array<string>` | textarea mỗi dòng một mục | Ví dụ danh sách URL video |

Kiểu `artifact` (vùng tải file) và `multiline` chỉ thêm khi có recipe cần tới.

Lỗi từ server trả về theo `validation_path`, và client gắn lỗi vào đúng trường đó. Nếu một lỗi không khớp trường nào thì hiện ở đầu form.

## API cần thêm

- `GET /api/admin/background/recipes`: trả danh sách gồm `id`, `version`, `title`, `description`, `params_schema` và `outline`. `outline` chỉ có `title`, `kind` (`tool`/`agent`) và `repeat`.
- `POST /api/admin/jobs` nhận thêm `recipe: { id, params }`. Nếu body có `recipe` thì không cần `goal`. Server tự sinh tiêu đề Job từ recipe và tham số. Idempotency key giữ như hiện tại.

## Thay đổi ở danh sách và chi tiết Job

- Danh sách Job: dưới mục tiêu hiện nhãn `recipe · <tên> v<version>`, giống cách nhãn Knowledge đang hiện.
- Chi tiết Job: thêm thanh tiến độ theo bước của recipe ở trên timeline hiện có. Thanh này cho biết bước hiện tại, số mục đã xong trong lô và bước dừng kèm `stop_reason`. Các lượt follow-up chạy agent loop vẫn hiện trong timeline như trước, có đánh dấu là lượt tự do.
- Ô follow-up giữ nguyên. Theo ADR 0140, follow-up trên Job recipe chạy agent loop tự do.

## Ngoài phạm vi v1

- Admin tạo hoặc sửa recipe trên giao diện.
- Lên lịch recipe ngay trong modal. Schedule `spawn_task` sẽ nhận `recipe` ở một bước sau.
- Lưu bộ tham số hay dùng thành preset.
