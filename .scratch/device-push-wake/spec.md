# Device Push Channel: đánh thức Device bằng MQTT để phát Reminder

Status: ready-for-agent

Quyết định kiến trúc: [ADR 0145](../../docs/adr/0145-device-push-channel-mqtt-wake-signal.md). Từ vựng: [GLOSSARY](../../GLOSSARY.md#reminders-and-direct-speech).

## Problem Statement

Người dùng đặt Reminder cho một Device, nhưng tới giờ Device không nói gì nếu lúc đó nó không ở trong một Voice Session. Device đang thức và chờ wake word không giữ kết nối nào với server, nên server không có cách gọi nó. Lời nhắc chỉ được phát khi có người tình cờ nói chuyện với Device sau đó, hoặc hết hạn sau 24 giờ mà không ai nghe thấy.

Admin tạo Reminder trên giao diện quản trị thấy trạng thái đồng bộ `pending` mãi mãi, vì phần firmware cache lịch chưa tồn tại, và không biết lời nhắc có tới hay không.

## Solution

Device đã đăng ký giữ một Device Push Channel tới broker MQTT của deployment. Khi Reminder đến hạn mà Device không có Voice Session, server gửi một Wake Signal. Device nhận tín hiệu, tự mở Voice Session, server phát Reminder bằng Direct Speech như hiện nay, Device phát xong thì gửi Playback ACK và đóng phiên.

Device lấy thông tin kết nối Device Push Channel trong phản hồi OTA lúc khởi động, không cần cấu hình tay. Admin thấy trạng thái `wake_on_due` thay cho `pending`. Deployment không bật tính năng thì mọi thứ giữ nguyên như hiện tại.

## User Stories

1. As a người dùng Device, I want Device tự nói lời nhắc đúng giờ khi tôi không đang trò chuyện với nó, so that tôi không bỏ lỡ việc đã hẹn.
2. As a người dùng Device, I want lời nhắc được phát bằng cùng giọng của trợ lý, so that tôi nhận ra đó là Device của mình.
3. As a người dùng Device, I want Device không bật mic khi nó tự mở phiên để đọc lời nhắc, so that nó không nghe lén hay trả lời nhầm tiếng động xung quanh.
4. As a người dùng Device, I want Device quay lại trạng thái chờ wake word sau khi đọc xong, so that tôi vẫn gọi được nó như bình thường.
5. As a người dùng Device, I want lời nhắc không cắt ngang khi tôi đang nói chuyện với Device, so that cuộc trò chuyện không bị gián đoạn.
6. As a người dùng Device, I want lời nhắc vẫn được phát sau khi Device mất mạng rồi có lại trong cùng ngày, so that sự cố mạng ngắn không làm mất lời nhắc.
7. As a người dùng Device, I want lời nhắc không bị phát lại nhiều lần khi Device đã phát xong, so that tôi không bị làm phiền.
8. As a người dùng Device, I want Device thử phát lại khi lần phát trước thất bại, so that lỗi tạm thời không làm mất lời nhắc.
9. As a người dùng Device, I want nhiều lời nhắc đến hạn cùng lúc được đọc lần lượt trong một lần thức, so that tôi nghe đủ mà Device không kết nối lại nhiều lần.
10. As a người dùng Device, I want Device không tự phát gì khi lời nhắc đã bị huỷ trước giờ hẹn, so that tôi không nghe nội dung đã bỏ.
11. As a người dùng Device, I want wake word vẫn hoạt động trong lúc Device giữ Device Push Channel, so that tính năng mới không làm giảm khả năng nghe lệnh.
12. As a người dùng Device, I want Device khởi động và trò chuyện bình thường khi broker MQTT không truy cập được, so that sự cố broker chỉ ảnh hưởng tới lời nhắc tự phát.
13. As an Admin, I want thấy trạng thái `wake_on_due` trên Reminder của Device dùng Device Push Channel, so that tôi biết lời nhắc sẽ được gọi tới khi đến hạn.
14. As an Admin, I want Reminder tạo trên giao diện quản trị cho Device đang rảnh vẫn được phát đúng giờ, so that tôi không cần Device đang trong phiên lúc tạo.
15. As an Admin, I want Device bị chặn không còn kết nối được Device Push Channel, so that chặn Device có hiệu lực trên mọi đường kết nối.
16. As an Admin, I want Device chưa hoàn tất đăng ký không nhận thông tin kết nối Device Push Channel, so that chỉ Device đã được chấp nhận mới vào broker.
17. As an Admin, I want lời nhắc đến hạn vẫn xuất hiện trong Notification Inbox như hiện nay, so that tính năng mới không thay đổi thông báo quản trị.
18. As an Admin, I want log ghi lại mỗi lần gửi Wake Signal cùng Device và Reminder, so that tôi truy được vì sao một lời nhắc đã hoặc chưa phát.
19. As an Admin, I want log phân biệt "không gửi được Wake Signal" với "đã gửi nhưng Device không kết nối", so that tôi biết lỗi nằm ở broker hay ở Device.
20. As a người vận hành, I want bật tắt tính năng bằng một mục cấu hình, so that tôi triển khai dần và tắt nhanh khi có sự cố.
21. As a người vận hành, I want server từ chối khởi động với cấu hình bật tính năng nhưng thiếu secret hoặc địa chỉ broker, so that lỗi cấu hình lộ ra ngay lúc khởi động.
22. As a người vận hành, I want server vẫn chạy và phục vụ thoại khi broker ngừng hoạt động, so that broker không trở thành điểm chết của cả hệ thống.
23. As a người vận hành, I want server tự nối lại broker sau khi broker khởi động lại, so that tôi không phải khởi động lại server.
24. As a người vận hành, I want broker chạy trong cùng bộ compose với server, so that triển khai và sao lưu không thêm quy trình riêng.
25. As a người vận hành, I want kết nối từ Device tới broker được mã hoá TLS trong môi trường production, so that thông tin đăng nhập không đi dạng rõ qua Internet.
26. As a người vận hành, I want đổi secret là đủ để vô hiệu toàn bộ mật khẩu Device cũ, so that tôi xử lý được sự cố lộ secret.
27. As a người vận hành, I want deployment không bật tính năng không phải chạy broker, so that cài đặt tối thiểu không nặng thêm.
28. As a chủ Device, I want Device chỉ đọc được topic của chính nó, so that Device khác không theo dõi được lịch thức của Device tôi.
29. As a chủ Device, I want Device không publish được lên bất kỳ topic nào, so that một Device bị chiếm quyền không đánh thức được Device khác.
30. As a chủ Device, I want Wake Signal không chứa nội dung lời nhắc, so that nội dung riêng tư không đi qua broker.
31. As a nhà phát triển firmware, I want thông tin Device Push Channel nằm trong một khối riêng của phản hồi OTA, so that nó không lẫn với cấu hình giao thức thoại.
32. As a nhà phát triển firmware, I want firmware cũ bỏ qua khối mới mà vẫn chạy, so that tôi nâng cấp server trước firmware.
33. As a nhà phát triển firmware, I want Wake Signal có mốc hết hiệu lực, so that Device bỏ qua tín hiệu retained đã cũ sau khi khởi động lại.
34. As a nhà phát triển firmware, I want Wake Signal có trường phiên bản, so that payload thay đổi sau này không làm hỏng firmware đang chạy.
35. As a nhà phát triển firmware, I want Device ngắt Device Push Channel trong lúc có Voice Session, so that Device ít RAM không phải giữ hai phiên TLS.
36. As a nhà phát triển firmware, I want một tín hiệu thừa chỉ dẫn tới một phiên rỗng tự đóng, so that tôi không phải xử lý trạng thái đặc biệt.
37. As a nhà phát triển server, I want mọi quy ước topic, Device Key, mật khẩu và payload nằm trong một module, so that OTA, scheduler và phân quyền broker không lệch nhau.
38. As a nhà phát triển server, I want test scheduler mà không cần broker thật, so that test chạy nhanh và ổn định trong CI.
39. As a nhà phát triển server, I want đổi loại broker sau này mà không sửa OTA hay scheduler, so that lựa chọn Mosquitto không bị khoá cứng.
40. As a nhà phát triển server, I want giao thức `sync_request`/`sync_ack` hiện có giữ nguyên, so that phần cache lịch đã hoãn làm tiếp được sau.

## Implementation Decisions

### Module `device_push` (mới, phía server)

- Một module sâu sở hữu toàn bộ quy ước của Device Push Channel. Interface gồm ba việc:
  - cấp thông tin kết nối cho một Device đã đăng ký (OTA handler gọi);
  - phát một Wake Signal tới một Device (`reminder_scheduler` gọi);
  - trả lời câu hỏi phân quyền của broker: được kết nối không, được đọc/ghi topic này không.
- Ẩn bên trong: tên topic, cách tính Device Key, cách sinh và kiểm mật khẩu, mã hoá payload, kết nối và nối lại broker, tài khoản dịch vụ của server.
- Một seam nội bộ cho việc publish, với hai adapter: client MQTT thật và bản ghi nhận dùng trong test (theo mẫu adapter ghi nhận của Notification Hub).
- Phát tín hiệu không chặn scheduler: lỗi broker được ghi log và trả về, scheduler tiếp tục vòng lặp.
- Module được dựng trong `AppState` chỉ khi tính năng bật; khi tắt, các nơi gọi coi như không có.

### Quy ước dữ liệu

- Device Key: 32 ký tự hex đầu của SHA-256 trên Device ID.
- Topic server→Device: `vela/v1/devices/{device_key}/signal`, QoS 1, retained.
- Payload Wake Signal: JSON gồm `v` (số nguyên, hiện là 1), `type` (`"wake"`), `id` (định danh Reminder), `expires_at` (Unix milli giây; bằng hạn chót phát của Reminder theo cửa sổ trễ 24h).
- Thông tin đăng nhập Device: username là Device ID nguyên bản; client ID là `vela-` cộng Device Key; mật khẩu là HMAC-SHA256 của secret trên chuỗi có tiền tố phiên bản và Device ID, mã hoá hex. Không có bảng, cột hay migration mới.
- Tài khoản dịch vụ của server: username cố định, mật khẩu dẫn xuất từ cùng secret với tiền tố khác; là chủ thể duy nhất được publish lên các topic `signal`.

### Phản hồi OTA

- Thêm khối tuỳ chọn `push` gồm `endpoint` (host:port công khai), `client_id`, `username`, `password`, `keepalive`, `topic`.
- Chỉ có mặt khi Device ở trạng thái đã đăng ký và tính năng bật. Device đang chờ đăng ký, bị chặn, hoặc deployment tắt tính năng thì không có khối này.
- Không bao giờ phát khoá `mqtt`: firmware dùng khoá đó để chọn giao thức thoại MQTT+UDP.

### Phân quyền broker

- Mosquitto gọi HTTP về server cho ba câu hỏi: xác thực người dùng, quyền superuser, quyền theo topic. Tích hợp dự kiến qua plugin `mosquitto-go-auth`; định dạng yêu cầu và mã trả về phải được xác nhận theo tài liệu plugin khi làm ticket hạ tầng.
- Xác thực Device: mật khẩu khớp HMAC **và** Device Admission trả về đã đăng ký. Device bị chặn hoặc không tồn tại bị từ chối.
- Quyền topic của Device: chỉ subscribe/đọc topic `signal` của chính Device Key đó. Mọi publish bị từ chối.
- Tài khoản dịch vụ: chỉ publish lên các topic `signal`.
- Endpoint chỉ trả cho phép/từ chối, không trả lý do. Không có superuser.

### `reminder_scheduler`

- Trong mỗi vòng, sau khi phục vụ các phiên sẵn sàng, scheduler lấy danh sách Device có Reminder đến hạn (hoặc đến mốc thử lại) mà không có phiên sẵn sàng, và phát Wake Signal cho từng Device.
- Mỗi Reminder chỉ phát một Wake Signal cho mỗi lần đến hạn hoặc mỗi mốc thử lại, không phát lại ở mỗi vòng quét.
- Device mở phiên sau Wake Signal được phục vụ bằng đường hiện có: scheduler được đánh thức khi Device kết nối, claim, Direct Speech, Playback ACK. Không thay đổi luật claim, lease, backoff hay hết hạn.
- Deadline ngủ của scheduler phải tính cả Reminder của Device không có phiên khi tính năng bật, để Wake Signal được phát đúng giờ.

### Trạng thái đồng bộ

- Thêm giá trị `wake_on_due`: áp cho Reminder đang chờ mà lẽ ra hiển thị `pending`, khi tính năng bật. Các giá trị `acked`, `out_of_window`, `unsupported` giữ nguyên nghĩa.
- Giao diện quản trị hiển thị nhãn tiếng Việt cho giá trị mới.

### Cấu hình

- Mục cấu hình mới cho Device Push: bật/tắt (mặc định tắt), endpoint công khai cho Device, địa chỉ broker nội bộ cho server, secret, keepalive.
- Bật mà thiếu endpoint, địa chỉ broker hoặc secret thì validation cấu hình thất bại lúc khởi động. Secret có thể đặt qua biến môi trường như các secret khác.

### Hạ tầng

- Thêm dịch vụ Mosquitto (kèm plugin xác thực HTTP) vào các bộ compose, bật theo profile để deployment không dùng tính năng không phải chạy.
- Production: listener TLS trên cổng công khai; dev/test: listener không TLS trong mạng nội bộ.
- Broker không cho kết nối ẩn danh và không lưu danh sách người dùng cục bộ.

### Firmware

- Lớp mới cho Device Push Channel, tách khỏi tầng giao thức thoại: đọc khối `push` từ phản hồi OTA, lưu vào vùng cấu hình riêng, kết nối, subscribe topic được cấp, tự nối lại.
- Nhận Wake Signal hợp lệ và chưa hết hiệu lực khi không có Voice Session: mở kênh âm thanh WebSocket với hello khai báo khả năng reminders, không bật mic, không chuyển sang trạng thái nghe.
- Bỏ qua tín hiệu sai phiên bản, sai loại, hết hiệu lực, hoặc khi đang có phiên.
- Ngắt Device Push Channel khi mở Voice Session, nối lại khi phiên đóng.
- Phát âm thanh Direct Speech của Reminder; chỉ gửi `reminder.played` sau khi audio sink đã drain; lỗi decode/output/abort gửi `reminder.failed`.
- Chờ `delivery_result` đúng attempt rồi mới đóng phiên do Wake Signal mở; có thời gian chờ tối đa khi server không gửi gì.
- Không cache lịch, không ghi NVS cho Reminder, không hẹn giờ RTC.

## Testing Decisions

- Test tốt ở đây chỉ kiểm hành vi nhìn thấy từ bên ngoài: phản hồi HTTP, quyết định cho phép/từ chối, Wake Signal được ghi nhận, trạng thái Reminder. Không kiểm cấu trúc nội bộ của module hay thứ tự gọi hàm.
- Seam kiểm thử chính là tầng ứng dụng đã có (HTTP/WebSocket của server chạy trong test). Seam mới duy nhất là adapter publish ghi nhận của `device_push`.
- OTA: Device đã đăng ký nhận khối `push` đúng topic và thông tin; Device chờ đăng ký, bị chặn, hoặc tính năng tắt thì không; phản hồi không bao giờ có khoá `mqtt`. Prior art: các test đăng ký Device và admission hiện có.
- Phân quyền: mật khẩu đúng + Device đã đăng ký được chấp nhận; mật khẩu sai, Device bị chặn, Device lạ bị từ chối; Device đọc topic của mình được, topic Device khác không được, publish không được; tài khoản dịch vụ publish được. Thông tin lấy từ OTA phải qua được endpoint phân quyền (kiểm khớp đầu-cuối trong cùng một test).
- Scheduler: Reminder đến hạn không có phiên sinh đúng một Wake Signal; có phiên sẵn sàng thì không sinh; không phát lại ở vòng sau; phát lại ở mốc thử lại; Device kết nối sau tín hiệu thì Reminder được phát và hoàn tất bằng Playback ACK; publish lỗi không dừng scheduler. Prior art: các test Reminder của Admin API và test phiên Device đang hoạt động.
- Trạng thái đồng bộ: `wake_on_due` khi bật, `pending` khi tắt. Prior art: test Reminder của Admin API.
- Cấu hình: bật thiếu trường bắt buộc thì từ chối. Prior art: test validation cấu hình.
- Hạ tầng: kiểm tra bộ compose theo cách của feature docker-compose-validation; một kiểm tra khói kết nối/subscribe/publish với broker thật, chạy ngoài bộ test mặc định.
- Firmware: build thật và log phần cứng thật; harness server không thay cho bằng chứng audio sink và kết nối trên thiết bị.
- Mọi test server chạy qua script kiểm thử của repo, test mục tiêu trước rồi mới toàn workspace.

## Out of Scope

- Đánh thức Device đang Deep Sleep, cache lịch trên firmware, NVS snapshot, wake timer RTC (phần đã hoãn của ADR 0089).
- Gửi nội dung thông báo qua MQTT, hoặc phát nội dung không qua Direct Speech.
- Đọc Notification của Notification Hub (tác vụ hoàn tất, yêu cầu duyệt) trên Device.
- Kênh Device→server qua MQTT, trạng thái online/offline của Device qua LWT.
- Giao thức thoại MQTT+UDP của firmware.
- Siết xác thực endpoint OTA hoặc thay token WebSocket dùng chung.
- Giao diện quản trị để cấu hình Device Push; cấu hình nằm trong file cấu hình.
- Broker nhúng trong tiến trình server, cụm broker nhiều node.
- Cam kết phát đúng một lần; giới hạn của ADR 0090 giữ nguyên.

## Further Notes

- Tài liệu này thay mục "MQTT broker mới" trong phần ngoài phạm vi của spec [voice-reminders-direct-tts](../voice-reminders-direct-tts/spec.md) cho riêng tính năng này.
- Ticket firmware cũ [12](../voice-reminders-direct-tts/issues/12-firmware-online-ack.md) và [13](../voice-reminders-direct-tts/issues/13-firmware-offline-wake.md) giữ mở cho phần cache đã hoãn.
- Hai điểm chưa kiểm chứng, phải xác nhận trong ticket tương ứng: định dạng tích hợp của plugin xác thực HTTP cho Mosquitto, và việc client MQTT của firmware chọn TLS theo cổng.
- Độ trễ từ Wake Signal tới lúc bắt đầu phát chưa được đo; ticket nghiệm thu ghi số đo trên phần cứng thật.
- Thứ tự ticket: 01 → 02, 04, 07 song song; 03 sau 02; 05 sau 01 và 03; 06 sau 05; 08 sau cùng.
