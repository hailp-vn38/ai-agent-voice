# ADR 0074 — Enrollment Session qua WebSocket

## Status

Accepted — theo yêu cầu thiết bị chưa thêm trên web vẫn kết nối WS, hiển thị và
phát mã. Supersedes clause Unknown WS 403 và pending OTA luôn không token trong
ADR-0072/0073 khi enrollment enabled và transport websocket; database vẫn bắt buộc.

## Decision

Enrollment mặc định `transport="websocket"`. OTA Unknown trả URL/token transport
và không activation, để firmware thoát vòng OTA activation và có thể mở WS khi
wake/bấm nút. Voice bearer vẫn bắt buộc nếu config nonempty; token chung này không
phải credential phần cứng hoặc quyền dùng Agent. Sau bearer, Device registration
phân luồng: Registered đi qua profile/runtime admission đầy đủ; Unknown có một
Enrollment Session riêng; Blocked/DB failure fail closed. Mode ota giữ contract cũ.

Enrollment Session dùng cùng SQLite enrollment/code/TTL, không tạo Device, không
Voice SessionActor/provider runtime/transcript/MCP/ActiveTurnPermit. Nó gửi hello,
stt hiển thị mã, tts:start/sentence_start, raw Opus 24kHz mono 60ms và tts:stop sau
packet cuối. Asset PCM tiếng Việt preload/validate trước bind; encoder riêng chạy
trong bounded blocking work với concurrency 2. Connection cap riêng default 32;
absolute timeout 120 giây clamp TTL; replay qua listen có cooldown 60 giây.

Worker control-plane đọc một snapshot SQLite mỗi 2 giây, có cancellation và skip
missed ticks. Claim/manual-create thành công làm worker thấy Registered; writer
hủy prompt có thứ tự, gửi thông báo rồi close 1000. Kết nối tiếp theo mới acquire
profile/provider, không promote socket pending thành voice. Web claim không phụ
thuộc thông báo WS và không đồng nghĩa online. Client cần mở lại WS bằng wake/nút,
hoặc implement reconnect; server không giả định firmware tự reconnect.

## Consequences

Config enrollment cũ bật feature mà không transport sẽ dùng mode websocket; đặt
transport ota để giữ vòng activate. Deployment phải chuẩn bị 11 WAV clips trước
khi bật websocket enrollment; script tạo asset local bằng espeak-ng/ffmpeg hoặc
operator cung cấp recording riêng. Missing/invalid clip fail trước listener;
không synthesize bằng Agent TTS trong onboarding. Không thêm Redis/migration SQL,
không query DB trong Voice SessionActor và không nới Device/Agent voice admission.
