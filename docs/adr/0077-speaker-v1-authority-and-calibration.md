# ADR 0077 — Speaker V1 authority and calibration boundaries

## Status

Accepted design, 2026-10-07; cập nhật 2026-10-08. Xem [implementation guide](../speaker-identification-ws-logging-implementation-guide.md).

V1 dùng Speaker Authorization cho trò chuyện, tra cứu thông thường và điều khiển ít hậu quả như đèn hoặc âm lượng. Speaker Match có rủi ro replay/giọng tổng hợp, nên thao tác nhạy cảm như mở khóa hoặc truy cập dữ liệu riêng cần Independent Confirmation; chưa có cơ chế đó thì từ chối. Giữ Device và Admin authentication hiện có. V1 chặn tool nhạy cảm khi chưa có Independent Confirmation theo [ADR 0078](0078-agent-tool-allowlist.md); giọng match không cấp quyền bao quát cho tools.

Required xác minh mới ở từng voice turn, kể cả sau khi khóa Speaker trong Voice Session; chấp nhận từ chối lượt thiếu 2 giây tiếng nói thay vì kế thừa pass để hỗ trợ câu ngắn. Device control `abort` tiếp tục hoạt động theo protocol; câu “dừng” qua ASR vẫn chịu Speaker Gate. Observe là nhận dạng advisory theo từng Conversational Turn: match chỉ có thể điều chỉnh cách xưng hô trong prompt tạm thời của turn đó, không xác thực Device, không cấp tool/private-data permission và không được lưu hay tái dùng sang turn khác.

Mốc đầu gồm web enrollment và Observe từ ESP32 qua WS: provider/draft/thu mẫu → profile `pending` → Observe trên thiết bị. Quality-accepted quick enrollment chỉ tạo profile provisional; Observe không cần `calibration.json` hay qualification. Chỉ cho phép Required sau hiệu chỉnh bằng dữ liệu thực tế và đánh giá độc lập; browser holdout đạt không thay thế điều kiện này.

Speaker Match chỉ đại diện cho một cửa sổ audio tối đa 6 giây, không chứng minh cùng một người nói toàn bộ utterance. V1 chấp nhận giới hạn này trong phạm vi ít hậu quả; Required chưa xử lý đổi người giữa câu hoặc nói chồng giọng.

Profile deployment có revision, trạng thái `preliminary`/`qualified` và tham chiếu báo cáo. Người vận hành xác nhận qualification cho đúng embedding space, preprocessing, scoring parameters và điều kiện audio/tải đã đánh giá. Server kiểm khi bật Required và admission; thiếu, sai revision hoặc chưa qualified thì từ chối, Web không có bypass. Thay thông số đã qualification cần đánh giá lại; thu hồi qualification vô hiệu hóa Required sessions bị ảnh hưởng, không hạ policy xuống Observe. Cơ chế áp dụng deployment catalog được chốt tại [ADR 0079](0079-calibration-catalog-reload.md). Mục tiêu pilot dùng cận trên một phía 95% theo exact binomial, riêng bốn phép kiểm: FAR ≤ 1% ở 1:N và 1:1; genuine failure ≤ 10% ở 1:N và FRR ≤ 10% ở 1:1, trên audio đủ điều kiện. Genuine failure 1:N gồm reject và nhận nhầm người; misidentification báo riêng. Không tuyên bố bảo đảm đồng thời 95%. Corpus do operator quản lý ngoài server, tách các lần/phiên thu cho enrollment/calibration/held-out; protocol, số trial và stopping rule chốt trước evaluation. Chưa có ngưỡng nào được xác nhận đạt.

Operator review là điều kiện triển khai: chỉ đưa vào pilot Agent/Template có Persona, prompt, nguồn context và tool results phù hợp thông tin thông thường. Thay nguồn phải review lại; chưa review thì operator vô hiệu hóa hoặc loại khỏi pilot. V1 không xây eligibility workflow riêng và server chưa tự chứng minh/cưỡng chế eligibility nội dung. Speaker Gate/allowlist không bảo vệ dữ liệu riêng đã nằm trong prompt/context.

Người dùng vẫn có thể tự cung cấp thông tin riêng trong Dialogue History; review nguồn hệ thống không ngăn được điều này. V1 không bảo đảm bí mật hội thoại trước replay/giọng giả. Giữ cách ly history giữa Voice Sessions và không tự đưa lịch sử riêng từ nguồn khác vào pilot.

Phạm vi bổ sung phải vừa đủ với single-process homelab hiện tại: tái dùng provider manager, MCP discovery, SQLite/CAS, worker cleanup và Admin UI/API hiện có. Chưa xây credential operator riêng, eligibility workflow, tự phân loại nội dung, chống giả mạo, nhường slot theo thời gian hoặc hệ thống policy/scheduler tổng quát. Chỉ thêm các boundary cần thực thi contract đã chốt và qualification thực tế; không coi hướng phát triển sau V1 là acceptance bắt buộc.
