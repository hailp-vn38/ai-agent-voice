# 09: Phát hành thư viện, bỏ patch và chốt tài liệu

**What to build:** Server dùng các bản thư viện đã phát hành thay cho patch trỏ nhánh. Thuật ngữ và ADR phản ánh đúng thứ đã triển khai.

**Blocked by:** 01, 02, 03, 04, 05, 06, 07, 08

**Status:** resolved

- [x] Phát hành thư viện theo thứ tự trong `AGENTS.md`: `orchestrator-agent` 0.7.0, rồi `orchestrator-flow` 0.4.0. `llm-wiki-rs` không đổi.
- [x] Bỏ `[patch.crates-io]`, dùng version crates.io đã phát hành. `scripts/agent-test.sh full` pass.
- [x] Xóa nhánh local `spike/recipe-gateway` sau khi test ticket 01 thay thế spike.
- [x] `GLOSSARY.md` có thuật ngữ **Task Recipe**; ADR 0140 là Accepted; `docs/task-recipe-ui.md` mô tả UI hiện tại.
- [x] Ghi baseline trong [`docs/task-recipes-baseline.md`](../../../docs/task-recipes-baseline.md): 5 Task khớp heuristic playlist/YouTube, 4 hoàn tất (80%), trung bình 2,20 lượt LLM/Task, 0/5 có `STEP_LIMIT_REACHED`.

## Answer

Đã phát hành `orchestrator-agent` 0.7.0 trước `orchestrator-flow` 0.4.0, bỏ patch local và chạy pass toàn bộ workspace qua `scripts/agent-test.sh full`. Test toàn bộ package `voice-agent-server` cũng pass tại checkout có cấu hình cục bộ. Nhánh spike đã xóa; worktree thư viện gốc được chuyển về `main` và giữ nguyên. Tài liệu thuật ngữ, ADR, giao diện và baseline đã cập nhật. Baseline nhận diện playlist bằng chuỗi trong title/prompt vì database chưa có recipe ID có cấu trúc.
