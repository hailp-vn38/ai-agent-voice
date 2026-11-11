# 08: Prompt mặc định thôi yêu cầu nạp Required Job Skill

**What to build:** Prompt mặc định của `wiki_ingestor` và `wiki_curator` bỏ bước gọi `skill` cho các Required Job Skill của chúng, và nói rằng các workflow đó đã có sẵn bên dưới prompt. Phần còn lại của prompt giữ nguyên. Các prompt thực tế được định nghĩa trong `llm-wiki-rs`; `orchestrator-flow` re-export nên cập nhật mức phụ thuộc tối thiểu của thư viện. Server chuyển sang phiên bản thư viện mới.

Làm và test trong repo thư viện; thử từ server bằng `[patch.crates-io]` tạm rồi thay bằng phiên bản đã publish trước khi merge. **Publish lên crates.io cần người duyệt.**

Xem spec: [Required Job Skill](../spec.md#required-job-skill-adr-0143).

**Blocked by:** 06

**Status:** ready-for-human

- [x] Prompt mặc định của `wiki_ingestor` không còn yêu cầu gọi `skill` cho `wiki-ingest`, `wiki-taxonomy`, `wiki-page-format`
- [x] Prompt mặc định của `wiki_curator` không còn yêu cầu gọi `skill` cho `wiki-lint`, `wiki-query`
- [x] Lượt đầu của `wiki_ingestor` vẫn đọc index và nguồn song song như cũ
- [x] Một Sub-Agent có prompt riêng còn câu yêu cầu nạp Required Job Skill vẫn nạp được skill đó mà không bị từ chối (ticket 06 giữ Required Job Skill trong `allowed_skills` hiệu lực)
- [x] Deployment có file prompt mặc định cũ trên đĩa nhận prompt mặc định mới sau khi dựng app, nếu Admin chưa đặt prompt riêng
- [ ] Phiên bản mới được publish và ghi vào ticket này (cần người duyệt)

## Comments

- 2026-11-11: Implementation commits: `llm-wiki-rs` `3acca5a`; `orchestrator-flow` `46706a3` and `e0900a2`; server `cb24d6ba`, synced through integration `bb3d00be`. `llm-wiki-rs` full tests passed (20 unit + 2 integration); `orchestrator-flow` full tests passed (39 unit + 2 integration); server migration and ticket01 upgrade targeted tests passed. Scoped rustfmt and diff checks passed. crates.io publication and release version update remain pending reviewer approval.
