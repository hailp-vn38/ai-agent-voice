# ADR 0145 — Device Push Channel qua MQTT chỉ mang Wake Signal

Status: Accepted, 2026-11-13 — đã triển khai: mã server, hạ tầng và firmware đã merge; nghiệm thu trên phần cứng còn chờ ticket 08.
Amends: [ADR 0089](0089-reminder-snapshot-sync-and-device-wake.md).
Extends: [ADR 0088](0088-shared-session-direct-speech.md), [ADR 0090](0090-reminder-playback-ack-and-bounded-attempts.md), [ADR 0073](0073-required-database-and-device-admission.md).

Reminder đến hạn chỉ phát được khi Device đang có Voice Session: `reminder_scheduler` chỉ claim cho phiên sẵn sàng, còn Device rảnh (thức, giữ wake word, không có WebSocket) thì server không có đường nào gọi tới. ADR 0089 giải bài này bằng cache lịch trên firmware và một wake timer, nhưng phần firmware đó chưa được viết. Spec: [device-push-wake](../../.scratch/device-push-wake/spec.md).

Quyết định: thêm **Device Push Channel**, một kết nối MQTT thường trực từ Device tới broker Mosquitto, chỉ mang **Wake Signal**. Nhận Wake Signal, Device mở Voice Session; từ đó đường Direct Speech và Playback ACK hiện có chạy nguyên vẹn. Phần cache lịch và wake timer của ADR 0089 được hoãn.

## Decisions

- **Wake Signal không mang nội dung.** Payload chỉ có phiên bản, loại `wake`, định danh Reminder và mốc hết hiệu lực. Nội dung Reminder chỉ đi qua Voice Session đã admit, đúng nguyên tắc "firmware không tự phát nội dung cache" của ADR 0089.
- **Phát Wake Signal lúc đến hạn, không phải lúc lịch thay đổi.** `reminder_scheduler` phát khi một Device có Reminder đến hạn (hoặc đến mốc thử lại) mà không có phiên sẵn sàng. Mỗi Reminder phát một lần cho mỗi lần đến hạn/thử lại.
- **Một topic server→Device cho mỗi Device, QoS 1, retained.** Device kết nối lại sau mất mạng vẫn nhận tín hiệu cuối. Mốc hết hiệu lực nằm trong payload nên không cần xoá retained; tín hiệu thừa chỉ gây một lần mở phiên rỗng.
- **Topic dùng Device Key**, dạng băm hex của Device ID, vì Device ID hợp lệ hiện có thể chứa `/`, `+`, `#`.
- **Phản hồi OTA mang khối `push`, không dùng khoá `mqtt`.** Firmware coi `mqtt` là cấu hình giao thức thoại MQTT+UDP và sẽ bỏ WebSocket nếu thấy khoá đó. Khối `push` chỉ trả cho Device đã đăng ký và khi tính năng bật; firmware cũ bỏ qua khoá lạ.
- **Thông tin đăng nhập theo từng Device, không lưu trạng thái.** Mật khẩu là HMAC của Device ID với một secret của deployment. Không có bảng hay cột mới.
- **Broker hỏi server về quyền kết nối và quyền topic.** Mosquitto gọi HTTP về server; server kiểm HMAC rồi tra Device Admission trong SQLite. Device bị chặn mất quyền ở lần kết nối kế tiếp mà không phải đồng bộ danh sách sang broker. Device chỉ được đọc topic của chính nó; chỉ tài khoản dịch vụ của server được publish.
- **Một module sâu `device_push` sở hữu toàn bộ quy ước**: tên topic, Device Key, cách sinh mật khẩu, định dạng payload và kết nối broker. Interface gồm ba việc: cấp thông tin kết nối cho OTA, phát tín hiệu cho scheduler, trả lời câu hỏi phân quyền của broker. Bên trong có một seam publisher với hai adapter (client MQTT thật và bản ghi nhận cho test).
- **Tính năng tắt mặc định.** Khi tắt, phản hồi OTA, scheduler và trạng thái đồng bộ giữ nguyên hành vi hiện tại.
- **Firmware ngắt Device Push Channel trong lúc có Voice Session** và nối lại khi phiên đóng, để không giữ hai phiên TLS cùng lúc. Trong phiên, scheduler đã phát trực tiếp.
- **Trạng thái đồng bộ mới `wake_on_due`** thay cho `pending` khi tính năng bật và Device chưa có Device Cache ACK, để Admin không đọc nhầm là lời nhắc sẽ không tới.
- **Hoãn cache lịch và wake timer của ADR 0089.** Giao thức `sync_request`/`sync_ack` phía server giữ nguyên, không xoá.

## Considered Options

- **Giữ một WebSocket rảnh thường trực:** không cần broker, nhưng mỗi Device rảnh chiếm một Voice Session đã admit cùng actor và permit của nó, và mất tín hiệu khi đứt kết nối. MQTT có retained và QoS 1 sẵn.
- **Gửi nội dung thông báo qua MQTT:** bớt một lượt kết nối, nhưng tạo đường phát thứ hai ngoài Direct Speech, và OTA hiện chỉ nhận diện Device bằng header nên thông tin kết nối không đủ kín để chở nội dung.
- **Tín hiệu `sync_required` kèm Schedule Revision lúc lịch thay đổi:** vẫn đòi firmware cache lịch và tự hẹn giờ, tức vẫn phải làm phần firmware nặng nhất của ADR 0089.
- **Plugin Dynamic Security của Mosquitto:** không cần gọi HTTP, nhưng server phải đồng bộ danh sách Device sang broker và chặn Device trở thành thao tác trên hai nơi.
- **Nhúng broker vào tiến trình server:** bớt một container, nhưng chưa kiểm chứng độ chín và khả năng tuỳ biến xác thực; seam publisher cho phép đổi sau.
- **Dùng lại khoá `mqtt` của OTA:** làm firmware chuyển giao thức thoại sang MQTT+UDP.

## Consequences

- Deployment bật tính năng phải chạy thêm Mosquitto, mở một cổng MQTT/TLS ra ngoài và giữ thêm một secret.
- Reminder tới Device đang thức mà không cần firmware cache lịch. Đúng giờ phụ thuộc Device Push Channel đang kết nối lúc đến hạn; mất mạng thì phát trễ trong cửa sổ 24h của ADR 0090.
- Device đang Deep Sleep vẫn không được đánh thức. Trường hợp đó chờ phần đã hoãn của ADR 0089.
- Ai biết Device ID có thể lấy thông tin kết nối qua OTA, như với token WebSocket hiện nay. Thiệt hại giới hạn ở việc thấy thời điểm Wake Signal của Device đó.
- Mục "MQTT broker mới" trong phần ngoài phạm vi của spec [voice-reminders-direct-tts](../../.scratch/voice-reminders-direct-tts/spec.md) không còn đúng với tính năng này.
- Chi tiết tích hợp plugin xác thực HTTP của Mosquitto và hành vi TLS của client MQTT trong firmware chưa được kiểm chứng; ticket hạ tầng và firmware phải xác nhận trước khi chốt.
