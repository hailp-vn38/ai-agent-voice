# Đối chiếu QMD với Knowledge + Memory V3: xử lý Markdown

> Ngày kiểm tra: 2026-10-16. Nguồn QMD được khóa tại commit [`86d2365d`](https://github.com/tobi/qmd/commit/86d2365d17fc682e491c2c53d1e4a884aee8682a). Đối chiếu Rust tại commit [`63473a0b`](https://github.com/hailp-vn38/ai-agent-voice/commit/63473a0bb5ded790ce8b420132b80587e6928909) của branch `feat/knowledge-memory-v3-file-first-index`. Đây là **báo cáo đánh giá trước triển khai**. P1 lexical retrieval và P2 Markdown passages đã được hiện thực tiếp theo trên branch này; xem [ADR 0121](../adr/0121-qmd-inspired-markdown-passages.md) để biết thay đổi thực tế. P3 semantic retrieval vẫn chưa triển khai.

## Kết luận

**Nên sử dụng chọn lọc thuật toán của QMD, không chạy hoặc nhúng toàn bộ QMD như service Node/Bun trong Rust server.** Áp dụng sớm: lexical query parser an toàn, trọng số BM25 và đoạn trích khớp query có line-range/citation; tiếp theo: Markdown-aware chunking và ngữ cảnh tài liệu. Chỉ thêm embedding/RRF/reranker khi kiểm thử chất lượng truy xuất chứng minh cần thiết.

Giữ các ràng buộc đã chốt: (1) Markdown gốc là canonical, (2) main application SQLite + hai DB index riêng, (3) Wiki verified và Source kiểm chứng mới Agent-readable, (4) Memory owner-confirmed active mới recall được, (5) Job V3 chỉ gọi Core Plugin đọc, (6) không tạo QMD sidecar, DB thứ tư hoặc chuyển dữ liệu thẩm quyền sang QMD.

## Bằng chứng trực tiếp từ QMD

| Thành phần QMD | Cách hoạt động xác minh từ source | Ý nghĩa cho Rust V3 |
|---|---|---|
| Collections / filesystem scan | [src/store.ts:1844–2082](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/src/store.ts#L1844-L2082) sử dụng glob, ignore, từ chối symlink/path escape, kiểm tra root trước khi deactive tài liệu không còn tồn tại. | Áp dụng nguyên tắc full successful scan rồi mới xóa index; Rust đã có fail-closed, nên giữ chính sách **chặt hơn QMD** khi scan lỗi. |
| Đồng bộ tăng dần | [src/store.ts:1685–1842](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/src/store.ts#L1685-L1842) lưu mtime/size/hash với racy-mtime window 2 giây; hash không đổi thì không reindex body. | Hữu ích khi corpus lớn; chưa cần cache stat-only khi mỗi file Markdown hiện bị giới hạn 256 KiB và SHA-256 là gate toàn vẹn. |
| Index content + metadata | [src/store.ts:1237–1365](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/src/store.ts#L1237-L1365) lưu nội dung theo hash, mapping đường dẫn, FTS5, metadata, vector theo chunks. | Chỉ mượn mô hình projection/cache; **không copy content database làm canonical**. Stable Wiki ID khác docid hash của QMD. |
| FTS query parser | [src/store.ts:4540–4745](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/src/store.ts#L4540-L4745) biên dịch từ khóa thành query FTS5, hỗ trợ phrase, term prefix và compound terms như `DEC-0054`. | Giải quyết vấn đề hiện tại: cả input được ép thành một quoted phrase; chỉ nên port phân tích lexical an toàn, chưa cần mở negation cho Agent. |
| BM25 có trọng số | [src/store.ts:4710–4798](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/src/store.ts#L4710-L4798) ưu tiên filepath/title/body lần lượt 1.5/4/1, định thứ tự rồi mới trả kết quả. | Có thể tăng trọng số title so với body ngay trong `knowledge_index.db`. Stable ID không có ý nghĩa ngôn ngữ nên không cần điểm filepath tương tự. |
| Markdown chunking | [src/store.ts:201–475](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/src/store.ts#L201-L475) chấm điểm vị trí cắt tại heading, đoạn, list và code fence; có overlap, ngăn tách giữa surrogate pair. [Các hằng số](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/src/store.ts#L165-L173): 900 tokens/chunk, overlap 15% cho embedding. | Dùng làm cơ sở cho chunk **derived** với `start_line/end_line`, `heading_path` và `source_ref`; với Rust cần Unicode-safe character/byte offsets. |
| AST chunking | [src/store.ts:3739–3780](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/src/store.ts#L3739-L3780) + [README:675–705](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/README.md#L675-L705): tree-sitter chỉ tùy chọn cho code; Markdown mặc định regex-based. | Không thêm tree-sitter cho Markdown. Có thể dùng Markdown parser chuẩn khi cần xử lý fence `~~~`, heading, list chuẩn xác. |
| Ngữ cảnh dạng cây | [src/collections.ts:484–523](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/src/collections.ts#L484-L523) tìm context theo collection/path-prefix có mức đặc hiệu cao nhất; kết quả mang thêm context. | Phù hợp bổ sung `summary/context` theo topic hoặc page, nhưng **không** dùng quyền truy cập dựa vào path context. |
| Typed metadata | [src/metadata.ts:92–170](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/src/metadata.ts#L92-L170) và [README:938–1010](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/README.md#L938-L1010) phân tích `qmd.metadata` từ YAML frontmatter, có version và lọc theo metadata. | Có thể bổ sung tags/topic/lang làm metadata dẫn xuất; cấm metadata tự tuyên bố `verified` hoặc `confirmed` để vượt chính sách Core. |
| Hybrid + rerank | [src/store.ts:6264–6591](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/src/store.ts#L6264-L6591) và [README:471–568](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/README.md#L471-L568) FTS + vector + query expansion + RRF + rerank. | Không cần mặc định ở server cá nhân; chỉ triển khai sau khi đo độ phủ câu hỏi tiếng Việt và latency Job. |

## Đối chiếu với code hiện tại

- [database/knowledge_index.rs:128–213](https://github.com/hailp-vn38/ai-agent-voice/blob/63473a0bb5ded790ce8b420132b80587e6928909/crates/voice-agent-server/src/database/knowledge_index.rs#L128-L213): scanner đọc và hash Markdown mỗi lần reconcile; thay đổi sẽ được upsert qua một transaction, dữ liệu cũ chỉ bị retire sau quét đầy đủ. Đúng với file-first và nên giữ.
- [database/knowledge_index.rs:264–317](https://github.com/hailp-vn38/ai-agent-voice/blob/63473a0bb5ded790ce8b420132b80587e6928909/crates/voice-agent-server/src/database/knowledge_index.rs#L264-L317): FTS query đang là **một phrase trọn vẹn** (`"cụm từ người dùng nhập"`). Điều này có thể bỏ lỡ nội dung liên quan nếu câu hỏi không trùng nguyên văn, đặc biệt với câu hỏi dài từ Agent.
- Cũng tại search: `substr(f.body,1,600)` chỉ trả **600 ký tự đầu tài liệu**, không nhất thiết là đoạn chứa từ khóa. Đây là khoảng cách quan trọng với QMD best-chunk passage.
- [migrations/knowledge/0001_index.sql](https://github.com/hailp-vn38/ai-agent-voice/blob/63473a0bb5ded790ce8b420132b80587e6928909/crates/voice-agent-server/migrations/knowledge/0001_index.sql) dùng FTS5 `unicode61 remove_diacritics 2`, có title/body nhưng chưa có chunks/heading offsets.
- [services/knowledge/vault.rs:255–305](https://github.com/hailp-vn38/ai-agent-voice/blob/63473a0bb5ded790ce8b420132b80587e6928909/crates/voice-agent-server/src/services/knowledge/vault.rs#L255-L305) bảo toàn stable ID/revision, kiểm soát symlink và SHA-256; QMD không nên thay thế lớp chính sách file này.
- [services/knowledge/wiki.rs:213–261](https://github.com/hailp-vn38/ai-agent-voice/blob/63473a0bb5ded790ce8b420132b80587e6928909/crates/voice-agent-server/src/services/knowledge/wiki.rs#L213-L261) xác minh Source khi Agent đọc/trả kết quả; chunking và indexing phải sử dụng chính xác cùng gate này.
- [docs/knowledge/implementation-v3.md](https://github.com/hailp-vn38/ai-agent-voice/blob/63473a0bb5ded790ce8b420132b80587e6928909/docs/knowledge/implementation-v3.md): Knowledge và Memory có lifecycle riêng; không gộp hai index chỉ vì QMD dùng nhiều collections trong một DB.

## Phương án triển khai phù hợp

### P1 — cải thiện lexical retrieval (ưu tiên cao, ít thay đổi)

1. Tạo **LexQuery compiler** nội bộ tại database/search module: normalize Unicode, tách token an toàn theo tokenizer đang dùng, hỗ trợ phrase được yêu cầu rõ ràng; escape dấu nháy; giới hạn độ dài/số term. Input LLM **không được** thành raw FTS5 expression. Với câu nhiều từ, AND các term; có fallback OR hoặc OR-prefix hạn chế nếu AND không đủ kết quả. Không port nguyên parser negation/FTS operators của QMD ở phiên bản đầu.
2. Thử title weighting trong `bm25(kb_fts, 0.0, 4.0, 1.0)` (FTS table hiện gồm `stable_id UNINDEXED,title,body`). So sánh với baseline bằng tập câu hỏi tiếng Việt; chỉ thay đổi cấu hình khi có bằng chứng chất lượng.
3. Thay `substr(body,1,600)` bằng **query-relevant excerpt**; ở giai đoạn đầu có thể dùng SQLite FTS5 `snippet()` để hiển thị, nhưng citation chính xác phải dùng offset/line verified lấy từ canonical Markdown. Nếu chưa có offset tin cậy, trả `source_ref` + page ID và không bịa dòng.
4. Không thay đổi các operation `knowledge.search`, `knowledge.read`, `memory.recall`; các field mới nên là **optional** để không phá frozen Job tool contracts.

### P2 — Markdown-aware passages/chunks (ưu tiên trung bình)

Đưa `MarkdownChunker` dạng pure module dưới implementation tìm kiếm. Đầu vào là **body đã parse**, với canonical `stable_id`, `revision`, `content_sha256`, `status`, `source_ref`. Đầu ra:

```text
MarkdownChunk { stable_id, revision, ordinal, heading_path,
                start_byte, end_byte, start_line, end_line,
                chunk_sha256, text }
```

- Vị trí cắt ưu tiên heading → paragraph → list boundary; không cắt trong fenced code blocks; xử lý cả ba backticks, bốn backticks, tilde, CRLF, emoji, Unicode tiếng Việt. **Không sao chép trực tiếp regex fence của QMD** vì cần parser bao phủ đủ các biến thể Markdown.
- Kích thước khởi điểm có thể thử ~600–900 token (QMD mặc định 900), overlap 10–15%; phải kiểm tra tokenizer/chuỗi tiếng Việt và latency thực tế trước khi cố định. Chunk phải giới hạn số lượng và tổng byte để không vượt ngân sách Agent.
- Lưu `kb_chunks` + (nếu cần) `kb_chunks_fts` trong **knowledge_index.db**, migration riêng theo chuỗi SQLx hiện tại; index là projection, không thay thế tệp Markdown. Chỉ index **verified canonical Wiki** cho Agent; staged drafts/Raw Sources vẫn chỉ được quản trị.
- Khi publish revision mới: xây chunks ở memory -> transaction cập nhật row/chunk/FTS -> đánh dấu ready; nếu fail, index stale và lần đọc Agent phải fail closed. Stable ID của Wiki không thay đổi theo chunk hay content hash.
- `knowledge.search` trả best passage, `start_line/end_line`, `heading_path`, `source_ref`; `knowledge.read` vẫn đọc file canonical với line-range và authorization/Source check.
- Memory facts đang nhỏ và có lifecycle forget/confirm khác: **không chunk Memory mặc định**; chỉ nghiên cứu riêng khi thật sự có memory dài.

### P3 — tùy chọn semantic retrieval

Chỉ khi bộ đánh giá chỉ ra keyword search bỏ sót đáng kể (paraphrase, câu hỏi đa ngôn ngữ) mới thêm embedding cho **canonical verified Knowledge chunks**. Provider embedding được cấu hình riêng; index vector vẫn là projection có fingerprint theo model + tokenizer + chunker version. FTS chạy độc lập nếu model không sẵn; kết hợp BM25 và vector bằng RRF, thử rerank trong Job background, không đặt query expansion/reranking cồng kềnh lên voice hot path. Không cài QMD CLI/Node/Bun/sidecar vào Rust server.

## Những logic không nên port

- Không dùng QMD content hash docid làm Wiki identity, hoặc đưa tài liệu gốc vào SQLite làm source of truth.
- Không để collection path/global context quyết định security/ownership; quyền `verified`/`confirmed` phải được kiểm tra tại Rust Core.
- Không tự động deactivate tất cả trên failed/incomplete scan; giữ fail-closed logic đã có. Không dùng `mtime + size` để bỏ qua checksum Source bất biến.
- Không đưa QMD MCP server lên device wire; giữ Core Job Tool hiện tại và không tạo wire type mới.
- Không thêm thư viện tree-sitter, sqlite-vec, LLM expansion/reranker trong pha Markdown lexical đơn giản.

## Acceptance tests đề xuất

1. Câu hỏi tiếng Việt không trùng nguyên văn Markdown vẫn tìm được tài liệu đúng; không có FTS5 syntax injection; kiểm tra compound terms `DEC-0054`, `gpt-4` và dấu nháy.
2. Tiêu đề khớp mạnh được ưu tiên so với trang chỉ lặp từ khóa trong nội dung; benchmark trước/sau trên tập cố định.
3. Tài liệu dài có heading, danh sách, đoạn code ```/~~~, emoji, CRLF: chunk không mất/kép nội dung ngoài overlap đã công bố, offset và số dòng khớp canonical UTF-8.
4. Sau publish, chỉ revision verified xuất hiện trong index/chunks; khi sửa staged draft, Agent vẫn nhận revision trước đó; sau publish lại, chunks cũ biến mất atomically.
5. Tampered Source, mất nguyên vault, quét thiếu/không đầy đủ: trả trạng thái stale, không trả passage/citation giả; sau restore/rebuild tất cả phục hồi.
6. Memory unconfirmed/forgotten không xuất hiện trong bất cứ output Agent nào, dù query metadata hoặc full-text khớp.
7. Không có thay đổi hoặc biến động dữ liệu ở application SQLite ngoài policy/Job records sẵn có; index rebuild ra kết quả xác định từ cùng một bộ Markdown.

## Quyết định

**Chọn cải tiến từng lớp:** P1 Lexical + snippet trước; P2 Markdown-aware chunk và citation khi cần tài liệu dài; P3 hybrid semantic chỉ theo benchmark. Việc tham chiếu QMD không tạo yêu cầu thay thế Knowledge/Memory V3 đã chốt.

## Tham chiếu và bản quyền

- QMD upstream: [README](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/README.md), [src/store.ts](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/src/store.ts), [src/collections.ts](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/src/collections.ts), [src/metadata.ts](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/src/metadata.ts), [LICENSE](https://github.com/tobi/qmd/blob/86d2365d17fc682e491c2c53d1e4a884aee8682a/LICENSE).
- QMD công bố MIT License. Nếu sao chép/điều chỉnh mã nguồn thay vì tự viết lại thuật toán, phải giữ copyright và license notice theo điều khoản MIT. Báo cáo này chỉ tham chiếu phương pháp; chưa chép mã nguồn vào server.

## Trạng thái sau triển khai (2026-10-16)

- Đã triển khai trong Rust: bounded LexQuery compiler với AND/OR fallback, BM25 ưu tiên title, FTS5 snippets theo query.
- Đã triển khai: pure Markdown chunker (UTF-8 byte/line offsets, heading path, overlap, fenced code); `kb_chunks` + `kb_chunks_fts` là projection trong Knowledge SQLite, publication/rebuild có transaction và đổi indexer version.
- Đã cập nhật output `knowledge.search` thêm các trường optional `start_line`, `end_line`, `heading_path`; Memory vẫn không được chunk.
- Chưa triển khai: embeddings, hybrid RRF, rerank/query expansion. Những mục đề xuất P3 và các yêu cầu benchmark vẫn còn hiệu lực.

### Review bổ sung: passage snippet cho title-only matches

FTS5 `snippet(...,body-column,...)` trả nội dung body không liên quan khi từ khóa chỉ khớp title/heading. Retrieval hiện chọn `snippet(...,-1,...)`, để FTS5 tự lấy cột chứa match tốt nhất (title, heading hoặc body). Regression test tìm từ chỉ tồn tại trong title cho cả Wiki và Memory, ngăn Agent nhận excerpt không có evidence của query.
