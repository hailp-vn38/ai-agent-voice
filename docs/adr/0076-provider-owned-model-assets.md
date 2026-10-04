# ADR 0076 — Provider owns its model assets

## Status
Accepted. Supersedes [ADR 0044](0044-pinned-startup-model-preparation.md).

## Decision

Mỗi local provider sở hữu model của nó. Một module `assets.rs` cạnh provider khai báo
URL upstream đã pin, relative install path, revision và — khi upstream format cần
chuyển đổi — phép chuyển đổi đó. Không có manifest, không có model root cấu hình
được, không có network quyết định từ config.

Provider Asset Manager được đăng ký cùng Provider Adapter Registration. Khi Provider
Runtime Manager materialize một provider, nó gọi `ensure_assets()`; factory sau đó
resolve path và build runtime. `build()` không tải file.

### Integrity model

Một asset sẵn sàng khi nó là regular file và có kích thước lớn hơn 0. Không checksum,
không fingerprint, không parse ONNX ở bước chuẩn bị. Việc model có nạp được hay không
do runtime initialization xác nhận, và nó fail loud.

Lý do: checksum của model được publish cùng model từ cùng một nguồn, nên nó bảo vệ
trước download hỏng, không bảo vệ trước thay đổi trên đĩa. Download hỏng đã được
`.part` + atomic rename chặn. Đọc lại hàng gigabyte mỗi lần materialize để xác nhận
điều mà ta vừa tự tải là chi phí không đổi được lợi ích.

### Download safety

Download ghi vào `<target>.part`, kiểm kích thước, rồi rename nguyên tử. File final
không bao giờ tồn tại ở trạng thái dở dang. Mỗi asset có một striped lock nên hai
request materializing cùng provider không tải trùng file. Một lần `ensure_assets()`
thất bại thì lần sau thử lại được: không có final file và không có `.part` sót lại.

Server là một process homelab, nên không cần distributed lock.

### Versioning

URL upstream pin trong source code của provider qua `MODEL_REVISION`. Revision tham gia
Physical Resource Key, nên bump revision tạo runtime identity khác mà không phải đọc
model file nào. Không có runtime manifest và không có manifest SHA.

Khi upstream đổi model mà giữ nguyên tên file, developer chủ động đổi tên hoặc thêm
version vào path (ví dụ `models/TTS/zerotts/v2/`), hoặc xóa file cũ khi deploy. Không có
garbage collector; đây là trade-off được chấp nhận.

### Configuration

Operator không cấu hình được nơi model nằm, chúng tải từ đâu, hay offline mode.
`deployment.models.{root,offline,sources}`, `model_manifest`, `model_acknowledgements`
và `measured_manifest_sha256` đã bị xóa thay vì để lại làm dead config.

`num_threads` vẫn là runtime setting của server: local execution width là deployment-owned
và có giới hạn, không phải model identity.

### Provider-specific transform

Zipformer chuyển SentencePiece `bpe.model` thành `tokens.txt`; Kokoro VI trích tensor từ
voicepack archive thành `.bin`. Hai phép chuyển đổi này thuộc về cách provider đó tiêu thụ
model, nên chúng nằm trong `assets.rs` của provider chứ không phải một trường
`transform` tổng quát. File đã chuyển đổi mới là file được kiểm `exists + size > 0`;
nguồn `.pt`/`.bpe.model` chỉ tải khi file đích còn thiếu.

### Voice ownership

ZeroTTS có một physical engine phục vụ mọi logical voice, nên `ensure_assets()` đảm bảo
**toàn bộ** voice được hỗ trợ chứ không chỉ voice đang cấu hình. Voice catalog của
descriptor được project từ asset catalog lúc compile, nên voice mà Admin API quảng bá
chính là voice mà provider nạp được. Cùng một luận đó áp dụng cho Kokoro voicepack.

## Consequences

Server khởi động được khi model directory rỗng hoặc chưa có. Generic startup không scan
model, và cũng không materialize provider nào: nó chỉ dựng registry, Provider Runtime
Manager rồi bind. Một provider chỉ tải model khi nó thực sự được dùng, hoặc sớm hơn nếu
`preload = true`.

Điều này thay đổi hành vi so với trước: managed startup từng acquire sẵn bốn provider mặc
định theo `provider_defaults`, nên nó tải model ngay lúc khởi động. Bỏ việc đó là điều kiện
để một host chưa từng tải gì vẫn bind được. Hệ quả là request đầu tiên phải chờ materialize,
đổi lại startup không còn phụ thuộc trạng thái `models/`.

ZeroTTS materialize giờ báo `artifact_prepare_ms` khoảng 2 ms thay vì hash 1.2 GB ONNX.

Trade-off được chấp nhận: nếu file model trên đĩa bị sửa nhưng vẫn có kích thước lớn
hơn 0, asset layer sẽ reuse nó và runtime initialization fail sau đó. Đây là hành vi
được chọn để giữ hệ thống đơn giản và khởi động nhanh.

Một đổi model upstream không tự phát hiện được; nó cần developer đổi path hoặc tên
file. Không có integrity scan nền.

Benchmark qualification không còn phân biệt "model đã verify" với "model đã tải", vì
hai khái niệm đó giờ là một. `BenchmarkErrorCategory::model_preparation` và trường
`model_preparation_ms` được giữ nguyên để report JSON không đổi shape; `comparison_qualified`
bị xóa vì nó chỉ tồn tại để diễn đạt offline mode.
