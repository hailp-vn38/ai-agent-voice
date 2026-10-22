# Review triển khai Knowledge AI ingest

Mốc so sánh: `0f7941ea2b9f34b8cc2a32d5f5b31dcf09d6e0ec...423dad58`. Hai reviewer độc lập kiểm tra diff theo skill code-review. Spec: `docs/knowledge/ai-ingest-auto-publish-implementation-v3.md`, cùng điều chỉnh phạm vi của người dùng: chỉ thêm UI Knowledge Settings, giữ layout các page khác.

## Standards

Không phát hiện vi phạm bắt buộc trong AGENTS.md, CONTEXT.md hoặc quy ước issue tracker. Module chia theo trách nhiệm, test nằm riêng, SQL nằm dưới database, UI giới hạn ở Settings.

Một nhận xét thiết kế không chặn: **possible Primitive Obsession**. `publication_policy`, `state`, `actor` dùng chuỗi trong `processing/mod.rs`, `processing/proposals.rs` và các chuyển trạng thái tại `database/knowledge_processing.rs`. Enum nội bộ có thể giúp kiểm tra đầy đủ các trạng thái trong tương lai, trong khi giữ nguyên chuỗi SQL/JSON. Reviewer không xác định lỗi hành vi cụ thể. Giữ nguyên trong phạm vi này; schema SQL, validation và các test chuyển trạng thái hiện bảo vệ hợp đồng.

## Spec

Không có finding đủ chắc chắn trong diff.

- Không phát hiện yêu cầu thiếu ngoài các boundary đã ghi rõ pending. Review workspace và Source Analyze UI được loại khỏi scope theo yêu cầu người dùng; P4 được hoãn đúng tài liệu.
- Không phát hiện scope creep. Thay đổi UI giới hạn ở Knowledge Settings cùng kiểu API/i18n liên quan.
- Không phát hiện hành vi sai sau khi đối chiếu admission một provider, không fallback global, evidence/claim gate, graph integrity, metadata SQL, CAS và human drafts, publish intent/reconciliation/idempotency, authentication và settings.

Review mã nguồn không bổ sung bằng chứng qualification cho remote LLM, production-process crash hoặc deployed Runner. Các boundary này vẫn pending trong implementation record.

Tổng: Standards có 0 vi phạm bắt buộc, 1 góp ý thiết kế không chặn (chuỗi trạng thái); Spec có 0 finding.
