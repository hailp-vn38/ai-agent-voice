# Luồng sử dụng theo trang

## Kết nối và Overview

Nhập Admin token trong form Connect. `App.vue` giữ token, tải admin cache và mở RouterView. Nếu có fallback env/session token, app tự tải cache khi mount. Health probe không thay thế Admin authentication.

Overview hiển thị số Agent, Template, Provider, Device từ cache và `/ready`, `/api/admin/system` cho active sessions, loaded/failed providers, database status. Khi cache loading/error, counts là dấu gạch ngang. Counts hiện bị giới hạn bởi pagination của store, không phải tổng server.

Overview refresh chạy cả cache và server status. System tự probe `/health` mỗi 30 giây; readiness/system aggregates được tải ở Overview, không phải một polling loop đầy đủ trong System.

## Agents và Studio

Catalog Agent render cache; thêm Agent qua form. Agent Detail là `/agents/:agentId`, có bốn tab:

| Tab | Nội dung |
| --- | --- |
| Studio | Template switcher, pipeline, prompt/configuration, usage |
| External Tools | MCP bindings và observed External MCP tool reviews |
| Speakers | Policy `off`/`observe`, Speaker candidates |
| Devices | Device cards thuộc Agent, edit/delete |

Template switcher chỉ đưa các Template đã link Agent; query `?template=<key>` chọn một Template đã link, giá trị không hợp lệ fallback về default. Chọn Template để xem không tự đổi default. Set default là mutation riêng, reload links để lấy default/revision mới.

Tạo Template từ Agent tạo resource global, link Agent và set default. Link Template có sẵn không tạo bản sao. Copy Template tạo resource global mới, giữ cấu hình/provider bindings nhưng không link Agent. Một thay đổi prompt/Template dùng chung có thể ảnh hưởng nhiều Agent ở các phiên sau.

Pipeline chia VAD → ASR → LLM → TTS, có nhánh Vision về mặt UI. API chỉ bind bốn slot đầu; thao tác Vision chưa có write contract tương ứng. Provider node mở modal thông tin/edit; Provider catalog mở detail route riêng.

**Giới hạn đã biết:** chỉnh bindings qua Template edit form có thể chỉ đổi cache mà không ghi API; direct link/unlink ở pipeline là luồng khác. Device edit/delete trong tab Devices đang bị lỗi revision/identity. Xem findings 1–3 của [review](review.md).

## Templates

Catalog lọc theo tên, language, usage trên cache đã tải. Cards cho biết pipeline và Agent usage. Language là dữ liệu cấu hình, không bị dịch theo locale của UI.

Template Detail dùng chung AiPipeline và configuration panel, hiển thị Agent đang link với nhãn default/linked. Unlink Template default bị backend từ chối; đổi default trước. Unlink là gỡ relationship, không xóa Template global.

Delete chỉ được UI chặn trước theo Agent links và Device overrides trong cache. Backend còn kiểm provider bindings và history, nên Template chưa dùng bởi Agent vẫn có thể `template_in_use`. Unlink provider slots và các references cần thiết trước khi xóa; không có history purge UI để xử lý history references.

Copy giữ name tùy chọn, description, language, prompt và provider bindings. Nhiều API call được dùng để tạo resource/bindings; nếu lỗi ở giữa, resource đã tạo có thể vẫn tồn tại. Kiểm tra catalog trước khi retry để tránh tạo bản sao dư.

## Providers

Tìm theo tên, adapter, model; filter trạng thái và type trên cache. Usage được suy từ Template bindings, không persisted vào Provider. Badge `ready` trong catalog là mapping rút gọn, không phải bằng chứng desired runtime đã materialize; detail page có trạng thái/revisions đầy đủ hơn.

Tạo Provider qua ba bước:

1. Chọn type VAD/ASR/LLM/TTS và adapter từ server registry.
2. Điền tên, typed config theo descriptor, advanced fields, API key nếu adapter cần; có thể discover capabilities.
3. Xem cấu hình và chạy draft diagnostic khi hỗ trợ, sau đó lưu.

Tạo/lưu chỉ đổi desired configuration. Mở detail page để xem desired state, ready revisions, runtime match, failure code và `requires_restart`. Prepare chỉ bật khi Provider enabled và runtime cho phép; kết quả prepare/test không chỉnh runtime của Voice Session đang active.

| Test | Input/output UI |
| --- | --- |
| VAD | Saved Provider; JSON diagnostic result |
| ASR | Clip microphone tối đa 30 giây; text, language, elapsed, duration/RTF nếu có |
| LLM | Text tối đa 8192 ký tự; response text/elapsed |
| TTS | Text tối đa 4096 ký tự, optional voice/language; audio WAV playback |
| Speaker/Vision | Không có generic test trong ProviderTestPanel |

Cancel/đổi input/resource làm abort diagnostic và clear/invalidate kết quả. Kết quả test gắn với input/config vừa dùng; không suy ra readiness lâu dài.

Link Provider tới **Template**, type phải khớp slot; slot đã có Provider cần xác nhận replace. Edit Provider dùng chung có blast radius tới các Template dùng nó.

**Delete:** detail page chặn khi còn Template usage. Catalog có confirmation và unlink các binding trước khi xóa, không atomic. **Duplicate:** implementation hiện làm mất config riêng adapter và không sao chép stored credential; xem review trước khi dùng thao tác này.

## Devices và enrollment

Thêm Device trong Agent hoặc Devices catalog dùng claim mã đang hiển thị trên thiết bị, không tự nhập một row tùy ý:

1. Thiết bị đi vào enrollment flow và có mã kích hoạt.
2. Chọn Agent trong Devices catalog hoặc dùng Agent đang mở.
3. Nhập **chuỗi sáu chữ số**, giữ số 0 đầu; tùy chọn tên và Template override đã link Agent.
4. POST claim; thành công đưa Device vào cache/catalog, từ catalog điều hướng đến Device Detail.

Catalog tải nhiều page trước khi search/filter theo Agent và admission. Chạm giới hạn 100 page sẽ báo lỗi thay vì âm thầm coi danh sách là đủ. `Enabled/Disabled` là quyền admission quản trị; không chứng minh Device đang kết nối.

Device Detail route dùng **public `device_id`**, GET raw resource. Có thông tin timestamp nếu server trả, copy ID, effective Template và link Agent. Edit đổi name/description/Agent/override; đổi Agent clear draft override để không mang sang relationship không hợp lệ. Chọn Follow Agent gửi `template_key: null`.

Enable/disable và delete dùng raw revision, sau đó refresh cache. Delete có thể bị chặn bởi history. Không có telemetry last-seen/live WS; phần connection hiển thị unavailable.

Ưu tiên thao tác quản trị Device ở detail page hiện tại. Tab Devices của Agent dùng store với lỗi no-op sau load và sai identity sau claim; đây là workaround, không phải lỗi đã sửa.

## Speakers và recognition

Speakers catalog có paging 50 item, search trong trang đang xem. Có thể tạo profile metadata chưa có mẫu hoặc dùng thu mẫu để tạo Speaker có voiceprint.

Enrollment wizard:

1. Tải `/speaker-recognition` để biết availability, embedding space và min/max clip.
2. Cấp quyền microphone; capture audio, downmix/resample thành WAV mono PCM16 16 kHz.
3. POST capture; server trả accepted capture, quality, expiry.
4. Profile mới: nhập name/description rồi create-from-capture. Profile có sẵn: replace voiceprint dùng revision.

Workflow này khác [ADR 0082](../../../docs/adr/0082-unified-speaker-enrollment-default.md), vốn mô tả nhiều mẫu và independent holdout. Implementation hiện không có draft/sample/validate/finalize flow đó; xem chênh lệch domain trong review.

Default UI ban đầu là 5–10 giây nhưng limits server thay thế ngay khi summary thành công. Capture được chấp nhận không chứng minh voice authentication; đây là recognition advisory. Capture expiry/runtime incompatibility cần thu lại, không tái dùng artifact cũ.

Speaker Detail xem metadata, enabled state, voiceprints và compatibility với embedding space hiện tại; hỗ trợ edit, thu lại mẫu, purge voiceprint và delete. Purge gửi confirmation `PURGE_SPEAKER_VOICEPRINT`, giữ profile; delete cần unlink khỏi mọi Agent trước và backend dọn dữ liệu enrollment/voiceprint.

Agent Speakers chỉ có `off`/`observe`. Observe cá nhân hóa hội thoại theo từng turn, **không xác thực Device, không cấp tool permission, không cho truy cập dữ liệu riêng**. Policy/candidate changes áp dụng khi thiết bị kết nối lại theo contract hiện tại. `required`, calibration qualification và Independent Confirmation chưa được UI/API này triển khai.

## External MCP và tool review

MCP catalog quản lý Streamable HTTP Server với key, name, endpoint, auth none/bearer/custom header và timeouts. GET chỉ trả credential metadata mask; để trống input giữ credential. Custom header form chấp nhận tên lowercase chữ/số/gạch ngang. Endpoint display bỏ userinfo/query/fragment.

Connection test và tool discovery hỗ trợ draft hoặc saved Server. Discovery incomplete là lỗi; không approve tool từ một catalog chưa hoàn tất. Results có elapsed/tested time và dropped tools nếu có; chỉ là quan sát của lần probe, không phải connection status liên tục.

Agent External Tools tách hai bước:

1. Link/enable MCP Server binding, luôn gửi `required: false`.
2. Review observed tool theo server key, tên gốc, description, input schema, observed revision/fingerprint; approve, revoke hoặc mark sensitive.

Binding enabled không tự cho phép tool. Discovery diagnostic không tự approve. Tool observation không chứng minh backend đang online hoặc hành vi implementation vẫn giữ nguyên. Contract đổi/conflict cần reload và review lại; không nhập schema tùy ý để thay observed contract.

Tool sensitive bị chặn, không approve được trước khi clear sensitive. Speaker recognition không thay đổi policy này. UI không có Device tool approval/recovery. Delete MCP Server cần unlink Agent bindings trước.
