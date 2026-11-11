# 08: Prompt mặc định thôi yêu cầu nạp Required Job Skill

**What to build:** Trong repo thư viện orchestrator-flow, prompt mặc định của `wiki_ingestor` và `wiki_curator` bỏ bước gọi `skill` cho các Required Job Skill của chúng, và nói rằng các workflow đó đã có sẵn bên dưới prompt. Phần còn lại của prompt giữ nguyên. Server chuyển sang phiên bản thư viện mới.

Làm và test trong repo thư viện; thử từ server bằng `[patch.crates-io]` tạm rồi thay bằng phiên bản đã publish trước khi merge. **Publish lên crates.io cần người duyệt.**

Xem spec: [Required Job Skill](../spec.md#required-job-skill-adr-0143).

**Blocked by:** 06

**Status:** ready-for-agent

- [ ] Prompt mặc định của `wiki_ingestor` không còn yêu cầu gọi `skill` cho `wiki-ingest`, `wiki-taxonomy`, `wiki-page-format`
- [ ] Prompt mặc định của `wiki_curator` không còn yêu cầu gọi `skill` cho `wiki-lint`, `wiki-query`
- [ ] Lượt đầu của `wiki_ingestor` vẫn đọc index và nguồn song song như cũ
- [ ] Một Sub-Agent có prompt riêng còn câu yêu cầu nạp Required Job Skill vẫn nạp được skill đó mà không bị từ chối
- [ ] Deployment có file prompt mặc định cũ trên đĩa nhận prompt mặc định mới sau khi dựng app, nếu Admin chưa đặt prompt riêng
- [ ] Phiên bản mới được publish và ghi vào ticket này
