# 01: Vault: mở kho tri thức và đọc

Status: resolved
Blocked by: none
Spec: ../spec.md (user stories 2–4, 7, 9–15, 18–21)

## Mục tiêu

Tạo crate `wiki-core` (member của workspace gốc) có `Vault` mở được kho tri thức và đọc được qua interface đã chốt. Đây là tracer bullet đầu tiên, chưa có phần ghi.

## Phạm vi

- `Vault::open(config)`:
  - tạo cấu trúc `sources/` và `wiki/` khi thư mục trống;
  - tự đồng bộ Search Projection nếu file trên đĩa mới hơn index.
- `tree(scope)`, `read(path, window)`, `search(query)`, `graph(focus?)`, `status()`.
- `DocPath`: kiểu đã kiểm tra, gồm Space `raw`/`wiki`, đường dẫn tương đối, từ chối `..`, đường dẫn tuyệt đối và đường dẫn trỏ ra ngoài kho.
- `VaultError` (`NotFound`, `InvalidPath`, `Io`, `Index`) và `From<WikiError>`. Kiểu của `llm_wiki` không được lộ ra interface công khai.
- Bổ sung GLOSSARY: mục kho tri thức Wiki do `Vault` đại diện, phân biệt rõ với Memory Vault.

## Tiêu chí xong

- [x] Test trên kho thật trong thư mục tạm: mở thư mục trống → có cấu trúc chuẩn; mở kho có sẵn tài liệu → `tree`, `read`, `search` (gồm tìm tiếng Việt không dấu), `graph` trả kết quả đúng.
- [x] `DocPath` từ chối `../x`, `/etc/passwd`, symlink trỏ ra ngoài kho.
- [x] Các test đọc lấy từ test knowledge MCP hiện có (`wiki_tree`, `wiki_query`, `wiki_get`) được viết lại trên `Vault` và pass.
- [x] Chạy qua `scripts/agent-test.sh`.

## Comments

- Implemented in `crates/wiki-core`; tests cover empty/existing vaults, tree/read/search (Vietnamese without diacritics), graph/focus, invalid paths and escaping symlinks.
- `scripts/agent-test.sh package wiki-core` and `scripts/agent-test.sh check wiki-core` pass. Workspace `cargo fmt --all -- --check` still reports a pre-existing formatting diff in `crates/vela-server/src/app/admin/knowledge/pages.rs`; the new crate passes standalone rustfmt.
