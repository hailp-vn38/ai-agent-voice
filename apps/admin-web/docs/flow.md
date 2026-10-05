Nên xây dựng luồng gồm **3 bước tạo provider**, sau đó chuyển sang màn hình chi tiết để **gắn template, kiểm tra runtime và test**. UI phải phân biệt rõ **“đã lưu cấu hình”** và **“đã sẵn sàng sử dụng”**, vì server hiện cần restart để nạp provider mới.

Đề xuất dưới đây dựa trên API `dev-test` đã kiểm tra.

**1. Trang Providers**

Giữ trang danh sách, thêm nút **“Tạo provider”** ở góc phải.

Mỗi provider hiển thị:

| Nội dung | Cách hiển thị |
|---|---|
| Tên và key | Tên nổi bật, key nhỏ bên dưới |
| Loại | Badge `VAD`, `ASR`, `LLM`, `TTS` |
| Adapter | Tên implementation |
| Trạng thái cấu hình | Đang bật / Đã tắt |
| Trạng thái runtime | Chưa nạp / Đã nạp / Không khả dụng |
| Cấu hình đã thay đổi | Badge “Cần restart” khi runtime chưa khớp |
| Template sử dụng | Số lượng, mở danh sách khi bấm |

Nút **“Tạo provider”** mở drawer rộng khoảng **800–900 px** trên desktop; dùng toàn màn hình trên mobile. Drawer có nội dung cuộn và thanh hành động cố định phía dưới.

**2. Bước 1 — Chọn loại và adapter**

UI gồm:

- Bốn lựa chọn: **VAD, ASR, LLM, TTS**.
- Danh sách adapter tương ứng.
- Mỗi adapter có tên, mô tả ngắn và các khả năng đã biết như offline, streaming, ngôn ngữ.

API:

```http
GET /api/admin/provider-adapters?type=tts
GET /api/admin/provider-adapters/{adapter}
```

Sau khi chọn adapter, web giữ descriptor để dựng form ở bước tiếp theo.

Nút dưới drawer:

```text
Hủy                         Tiếp tục
```

**Chỉ bật “Tiếp tục” khi đã chọn adapter.** Khi đổi adapter, xóa các trường cấu hình của adapter cũ để tránh gửi nhầm dữ liệu.

**3. Bước 2 — Nhập thông tin và cấu hình**

Chia form thành hai nhóm.

| Nhóm | Trường cần có |
|---|---|
| Thông tin provider | Tên hiển thị, key |
| Cấu hình adapter | Dựng theo `config_schema.fields` |
| Credential, nếu cần | Tên biến môi trường trong `secret_ref` |

Ví dụ TTS ZeroTTS:

| Trường | UI |
|---|---|
| Tên | Text input: “Giọng Mai Chi” |
| Key | Text input: `tts_maichi` |
| Model | Select |
| Voice | Select |
| Language | Select |
| Threads | Number input |
| Delivery mode | Select `stream` / `file` |
| Preload | Switch trong nhóm “Nâng cao” |

Web có thể gợi ý key từ tên, nhưng cho người dùng sửa. Validate key theo quy tắc hiện tại: bắt đầu bằng chữ thường, chỉ chứa chữ thường, số và `_`, tối đa 64 ký tự.

Dựng control theo descriptor:

| Metadata | Xử lý trên web |
|---|---|
| `type=string` | Text input |
| `type=integer` | Number input |
| `type=boolean` | Switch |
| `type=select` và `enum_values` | Select từ các giá trị được cung cấp |
| `enum_source=models` | Lấy lựa chọn từ capabilities model |
| `enum_source=voices` | Lấy lựa chọn từ capabilities voice |
| `enum_source=languages` | Lấy lựa chọn từ capabilities language |
| `required`, `minimum`, `maximum`, `max_length` | Validate và hiển thị giới hạn |

Với adapter hỗ trợ bootstrap discovery, gọi API trước khi tạo:

```http
POST /api/admin/provider-adapters/{adapter}/capabilities/discover
```

Ví dụ:

```json
{
  "selection": {
    "model": "zerotts_default"
  }
}
```

Khi đổi model, cập nhật danh sách voice và xóa voice đã chọn nếu không còn hợp lệ. Trong lúc lấy dữ liệu, hiển thị loading ngay tại control; nếu lỗi, có nút **“Thử lại”**.

Với adapter không hỗ trợ discovery, dùng dữ liệu static hoặc input theo descriptor. Ví dụ ChillAudio hiện nhập voice bằng chuỗi.

Không dùng `/providers/{key}/capabilities` ở bước này vì provider chưa có runtime.

Với credential, label nên là **“Biến môi trường chứa API key/token”**, kèm ví dụ `OPENAI_API_KEY`. Form gửi tên biến qua `secret_ref`; giá trị secret được thiết lập trên server.

**4. Bước 3 — Kiểm tra và tạo**

Hiển thị bản tóm tắt dễ đọc:

| Nội dung | Ví dụ |
|---|---|
| Provider | Giọng Mai Chi |
| Key | `tts_maichi` |
| Loại / Adapter | TTS / ZeroTTS |
| Model | ZeroTTS Default |
| Voice / Language | Mai Chi / Vietnamese |
| Cấu hình nâng cao | 4 threads, stream, preload tắt |

Thêm dòng thông báo ngắn:

> Provider sẽ được lưu vào hệ thống. Sau đó, gắn vào template và restart server để sử dụng.

Có mục thu gọn **“Xem JSON gửi lên”** để hỗ trợ kiểm tra, nhưng không bắt người dùng nhập JSON.

Nút:

```text
Quay lại                    Tạo provider
```

Khi bấm tạo:

```http
POST /api/admin/providers
```

```json
{
  "key": "tts_maichi",
  "name": "Giọng Mai Chi",
  "type": "tts",
  "adapter": "zerotts_onnx",
  "config_json": {
    "model": "zerotts_default",
    "voice": "maichi",
    "language": "vi-VN",
    "num_threads": 4,
    "preload": false,
    "delivery_mode": "stream"
  }
}
```

Trong lúc gửi, khóa nút tạo để tránh request trùng. Nếu lỗi, giữ nguyên form và đưa người dùng về trường liên quan khi xác định được nguyên nhân.

Sau `201 Created`, đóng drawer và mở trang chi tiết provider.

**5. Trang chi tiết sau khi tạo**

Phần đầu hiển thị tên provider, loại, adapter và trạng thái:

> **Đã tạo — chưa nạp runtime**

Bên dưới đặt khối **“Đưa provider vào sử dụng”** với các bước:

| Bước | UI và hành động |
|---|---|
| Gắn template | Chọn template và bấm “Gắn provider” |
| Kiểm tra liên kết agent | Hiển thị template đang được agent nào sử dụng |
| Nạp runtime | Thông báo cần restart server |
| Kiểm tra lại | Nút gọi API lấy trạng thái |
| Test | Mở công cụ test khi runtime sẵn sàng |

Chọn template rồi lấy revision:

```http
GET /api/admin/templates/{template_key}
```

Gắn provider đúng vị trí:

```http
PUT /api/admin/templates/{template_key}/providers/tts
If-Match: "<template_revision>"
```

```json
{
  "provider_key": "tts_maichi"
}
```

Nếu vị trí TTS đã có provider, hiển thị rõ:

> TTS của template sẽ đổi từ “Provider A” sang “Giọng Mai Chi”.

Nếu template chưa gắn vào agent, cung cấp lựa chọn agent và gọi:

```http
PUT /api/admin/agents/{agent_key}/templates/{template_key}
If-Match: "<agent_revision>"
```

**Template phải được gắn vào agent đang bật để provider được đưa vào kế hoạch nạp**, trừ trường hợp provider đã được tham chiếu bởi server defaults. Chỉ gắn vào một template chưa được agent sử dụng có thể vẫn khiến runtime giữ trạng thái `not_loaded` sau restart.

Không cần bắt buộc chọn template làm mặc định để test: template phụ đang bật và đã gắn vào agent đang bật cũng được thử nạp khi startup.

**6. UI trạng thái và công cụ test**

Nút **“Kiểm tra lại”** gọi:

```http
GET /api/admin/providers/{provider_key}
```

| Trạng thái API | UI đề xuất |
|---|---|
| `not_loaded` | “Chưa nạp”; hiển thị hướng dẫn gắn template và restart |
| `unavailable` | “Không khả dụng”; cần kiểm tra cấu hình/runtime trên server |
| `loaded` và khớp desired | “Sẵn sàng”; bật test |
| `loaded` nhưng không khớp desired | “Cấu hình mới chưa áp dụng”; yêu cầu restart trước khi test cấu hình mới |

API hiện chưa có endpoint restart trong luồng đã kiểm tra. Vì vậy, web hiển thị hướng dẫn restart và nút **“Kiểm tra lại”**.

Công cụ test thay đổi theo loại:

| Loại | UI test |
|---|---|
| TTS | Textarea, voice/language nếu hỗ trợ override, nút tạo audio, audio player và tải WAV |
| LLM | Ô nhập câu hỏi, nút test, vùng hiển thị câu trả lời |
| ASR | Upload WAV, nút nhận diện, kết quả transcript |
| VAD | Nút chạy kiểm tra frame im lặng, hiển thị probability và thời gian |

Ví dụ TTS gọi:

```http
POST /api/admin/providers/{provider_key}/test/tts
```

Response là `audio/wav`; web nhận dưới dạng `Blob` và đưa vào audio player.

**Thứ tự triển khai cho agent web:**

1. Thêm API client và kiểu dữ liệu descriptor, provider, runtime.
2. Xây drawer tạo provider ba bước.
3. Dựng form từ descriptor và hỗ trợ discovery.
4. Tạo provider rồi chuyển sang trang chi tiết.
5. Thêm giao diện gắn template và kiểm tra liên kết agent.
6. Hiển thị trạng thái runtime, hướng dẫn restart và nút kiểm tra lại.
7. Thêm công cụ test theo loại provider.

Tiêu chí hoàn thành chính là: **người dùng tạo được provider mà không nhập JSON, biết provider đã lưu hay đã chạy, và hiểu bước tiếp theo để dùng nó trong agent.**