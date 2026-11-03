# 09: Phát hành thư viện, bỏ patch và chốt tài liệu

**What to build:** Server dùng các bản thư viện đã phát hành thay cho patch trỏ nhánh. Thuật ngữ và ADR phản ánh đúng thứ đã triển khai.

**Blocked by:** 01, 02, 03, 04, 05, 06, 07, 08

**Status:** ready-for-agent

- [ ] Phát hành thư viện theo thứ tự trong `AGENTS.md`: `llm-wiki-rs` (nếu có thay đổi), `orchestrator-agent`, rồi `orchestrator-flow`.
- [ ] Bỏ `[patch.crates-io]` và dùng version đã phát hành. Test workspace pass qua `scripts/agent-test.sh`.
- [ ] `GLOSSARY.md` có thuật ngữ **Task Recipe**. ADR 0140 chuyển sang Accepted. `docs/task-recipe-ui.md` khớp với UI đã làm.
- [ ] Ghi số liệu nền về độ ổn định từ `tasks.db`: tỉ lệ hoàn tất của Task playlist, số lượt LLM mỗi Task, và tần suất `STEP_LIMIT_REACHED`.
