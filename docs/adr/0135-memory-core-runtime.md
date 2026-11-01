# ADR 0135 — Memory Core runtime: vault riêng theo Device, một đường ghi duy nhất

Status: Accepted, 2026-11-01.
Extends: [ADR 0119](0119-memory-wiki-separated-ownership.md), [ADR 0127](0127-orchestrator-flow-task-and-wiki-runtime.md).

Màn Memory hiện chỉ là tên khác của Wiki: Memory Fact nằm lẫn với Wiki Page, đọc được qua công cụ Wiki và Knowledge MCP, quên không để lại dấu vết, và Voice Session lẫn Background Task đều không dùng Memory. `voice-agent-server` dựng một module sâu `MemoryCore`, là đường duy nhất đọc và ghi Memory, với Memory Vault Markdown tách khỏi Wiki và phân vùng theo Device. Mọi thứ mặc định tắt (`[memory].enabled = false`, `devices.memory_mode = 'off'`), Device hiện có không đổi hành vi cho tới khi Admin bật từng Device.

ADR này chốt quyết định ở mốc 1 của [spec](../../.scratch/memory-core/spec.md); code đến theo các mốc 2 đến 7. Trước mốc 4, màn Memory của Admin vẫn chạy trên Wiki.

## Quyết định so với thiết kế ban đầu

Đối chiếu [thiết kế](../memory-core-integration-design.md) với code cho ra các quyết định sau, đánh số theo [hướng dẫn triển khai](../memory-core-implementation-guide.md).

- **D1. Không dùng `WikiEngine` cho Memory.** `open_with_config` luôn dựng cấu trúc thư mục Wiki, và FTS lưu body thô nên không gấp được `đ` lúc index, tức là `dat` không khớp `đặt`. Thay bằng một bảng FTS5 qua `sqlx`, tokenizer `unicode61 remove_diacritics 2`, mở kết nối theo từng lời gọi, văn bản và truy vấn cùng qua một hàm `normalize`. Vẫn dùng lại `fold_vietnamese`, `slugify`, `parse_frontmatter`. Index là seam nội bộ, nên đổi lại về sau chỉ đụng một thành phần.
- **D2. Allowlist của Agent `Primary` lấy từ `default_allowed_tools`.** Nhánh `Primary` đang ép chỉ có `subagent`, nên sửa `specs.rs` một mình không có tác dụng. Hai tool đọc của Task luôn được đăng ký, kể cả khi Memory tắt, vì allowlist có tool chưa đăng ký sẽ làm hỏng `build()`; khi đó tool trả `memory_disabled`.
- **D3. Cổng ý định theo từng hành động.** `MemoryActor::VoiceUser` mang `ExplicitIntent` gồm ba cờ `remember`, `correct`, `forget`. Một `bool` chung sẽ cho lượt "nhớ giúp X" vượt cổng khi gọi Quên.
- **D4. Interface có sáu method.** Thêm `reconcile_all`, `MemoryChange::SetProfile`, `MemoryChange::PurgeScope` và `MemoryScope::deployment()`, vì hồ sơ deployment, xóa vault và báo thư mục mồ côi đều cần một đường đi qua `MemoryCore`.
- **D5. `Correct` nhận thêm `title` và `hook` tùy chọn.** `hook` là dữ liệu của Memory Summary, và UI cần sửa được cả hai.
- **D6. `content_hash` nằm trong frontmatter.** `reconcile` cần mốc để phát hiện sửa ngoài luồng, mà index thì xóa được nên không giữ mốc đó được.
- **D7. Khóa HMAC của Forget Tombstone là file 32 byte trong `_deployment`,** quyền `0600`, tạo bằng `ring` lúc `open`. Mất khóa thì tombstone cũ hết tác dụng, nên backup thủ công phải gồm cả file này.
- **D8. Qua điểm commit thì `apply` luôn trả `Ok`.** Dọn dẹp lỗi thì đặt `cleanup_pending: true`, `reconcile` sẽ hoàn tất. Báo lỗi sau khi fact đã bị quên sẽ khiến model nói sai và người dùng nói lại rồi gặp `memory_not_found`. Publish `MemoryContext` trước khi ghi `MEMORY.md`.
- **D9. Vault chỉ bị xóa sau khi `DELETE` Device thành công.** `DeleteSpec::DEVICE` từ chối với `device_in_use` khi còn lịch sử, và quy tắc này giữ nguyên. Đường import cố ý không purge, để lần import sai vẫn khôi phục được.
- **D10. Id là `fact_<uuid v4 simple>`.** Workspace không có `ulid`. Thêm `serde_yaml` làm dependency trực tiếp. Test dùng `std::env::temp_dir()` cộng uuid, không có crate `tempfile`.
- **D11. Suy giảm được báo bằng sự kiện `tracing` có `event`, chỉ gồm metadata.** Repo không dùng crate `metrics` cho việc này, và `RuntimeCounter` chỉ thuộc provider runtime.

## Bổ sung của spec so với hướng dẫn

- **`Correct` có body mới khớp Forget Tombstone thì gỡ tombstone,** cùng lý do với `Remember`: người dùng đã nói rõ. Nếu không gỡ thì `Correct` trả thành công nhưng `recall` lại lọc mất fact, phá bất biến "`apply` thành công thì đã có hiệu lực", và `reconcile` lần sau sẽ xóa fact. File sửa ngoài luồng mà khớp tombstone thì vẫn bị `reconcile` xóa, vì sửa tay không phải ý định rõ ràng qua hệ thống.
- **`don't forget` bị loại khỏi nhóm `forget`** trước khi khớp, cùng với `đừng quên`. Hướng dẫn chỉ nêu dạng tiếng Việt, nhưng `forget` tiếng Anh gặp đúng bẫy đó. Cả hai mở cờ `remember`.

## Đối chiếu ADR

- **ADR 0119: hiện thực hóa, lệch một phần có chủ đích.** ADR nói hai domain dùng lại phần cài đặt Markdown và index. Memory dùng lại helper nhưng tự viết index (D1), vì `WikiEngine` không gấp được `đ`. Tách sở hữu, `MEMORY.md` là dẫn xuất, và "Inference không âm thầm thành Memory" đều giữ nguyên.
- **ADR 0008: _lệch tinh thần_.** ADR coi Device ID chỉ là metadata, còn Memory dùng nó làm khóa phân vùng. Server chỉ có một token toàn cục, nên client trên LAN biết `device_id` có thể nghe được Memory của Device đó. **Phân vùng theo Device không phải bảo mật.** Chấp nhận trên trusted LAN, nói rõ trên UI, và phải xem lại nếu triển khai ngoài LAN.
- **ADR 0024: phạm vi ngoại lệ.** Memory Fact là dữ liệu người dùng hoặc Admin chủ động lưu, nên được lưu bền trong Memory Vault và gửi tới LLM provider trong prompt. Ngoại lệ chỉ gồm nội dung fact đã được lưu có chủ đích, không mở rộng sang transcript, audio hay nội dung suy ra từ hội thoại. Log và sự kiện vẫn chỉ có metadata: không có tham số tool, không có nội dung fact, thư mục Device dùng hash thay cho Device ID nguyên bản, và `tombstones.json` không chứa title hay body.
- **ADR 0131: Memory Vault nằm ngoài backup.** Bundle chỉ giữ `memory_mode`, không gồm Memory Vault, và giữ nguyên `BUNDLE_SCHEMA_VERSION = 1` vì codec so sánh tuyệt đối. Memory Vault phải sao lưu riêng, gồm cả khóa tombstone. Trước Giai đoạn 4 phải chọn: mở rộng ADR 0131, hoặc ghi vào tài liệu vận hành.
- **ADR 0127:** mở rộng thêm một vault, `WikiEngine` vẫn sở hữu Wiki Vault.
- **ADR 0084, 0002, 0005, 0078:** tuân thủ. Actor chỉ đọc snapshot, ghi chạy ngoài actor, và `memory_mode` là cổng riêng cho builtin.
- **ADR 0120:** đã bị thay thế, không khôi phục `memory_index.db`.

## Consequences

- Fact cũ trong `wiki/entities/` không phân biệt được với trang của `wiki_ingestor`, nên ở lại Wiki như Knowledge. Từ mốc 4 chúng không còn hiện trên màn Memory; Admin chép thủ công khi cần.
- Quên chỉ có tác dụng trong Memory Vault. Lượt đang chạy, Dialogue History, `history_messages`, transcript trong `tasks.db` và phía LLM provider không bị xóa, nên kết quả trả về phải nói thật các giới hạn này. Tombstone khớp chính xác sau chuẩn hóa, nên diễn đạt khác sẽ lọt qua.
- `device:<id>` của Task là id dòng. Khôi phục bundle sang máy mới và chép kèm `tasks.db` có thể làm task cũ trỏ sang Device khác và đọc Memory của Device đó. Tài liệu vận hành phải cấm tổ hợp này, hoặc xóa task và lịch cũ khi khôi phục.
- Tự học ghi thẳng (`auto`) vẫn mâu thuẫn với ADR 0119 và cần ADR mới nếu muốn mở lại. Learner `suggest` và Memory Proposal cần spec riêng.
