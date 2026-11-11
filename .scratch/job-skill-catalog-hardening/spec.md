# Skill Catalog bền vững: nâng cấp Built-in, Required Job Skill và yêu cầu tool

Status: ready-for-human

Quyết định kiến trúc (đều Accepted ngày 2026-11-12):

- [ADR 0142](../../docs/adr/0142-single-skill-discovery-and-builtin-skill-upgrades.md): một cách quét Skill Catalog duy nhất và nâng cấp Built-in Job Skill.
- [ADR 0143](../../docs/adr/0143-required-job-skills-preloaded-into-agent-prompts.md): Required Job Skill nạp sẵn vào system prompt.
- [ADR 0144](../../docs/adr/0144-job-skill-tool-requirements.md): Job Skill khai báo tool nó cần.

Ticket: [issues/](issues/). Ba nhóm ticket (01–05, 06–08, 09–12) độc lập với nhau; ticket 13 làm sau cùng.

Sửa đổi [ADR 0128](../../docs/adr/0128-admin-managed-job-skill-catalog.md):

- ADR 0128 nói Built-in Job Skill được seed lúc khởi động. Spec này thêm: bản trên đĩa chưa bị Admin sửa được **thay bằng bản mặc định mới** khi server nâng cấp.
- ADR 0128 nói một `SKILL.md` hỏng không được chặn `reconfigure`. Hiện điều này không đúng trong mọi trường hợp; spec này làm cho nó đúng, và mở rộng sang cả lúc khởi động.
- ADR 0128 ghi 6 Built-in Job Skill. Thực tế là 11, và spec này thêm một cái nữa.

## Problem Statement

Admin sửa và gán Job Skill cho Sub-Agent qua Admin Web, nhưng hệ thống có bốn chỗ làm Admin bất ngờ:

1. **Một file skill hỏng làm sập cả server.** Một `SKILL.md` sai định dạng, một thư mục rỗng hay một thư mục tên sai trong thư mục skill khiến server không khởi động được. Khi server đang chạy, một Built-in Job Skill hỏng trên đĩa làm mọi lần lưu cấu hình Sub-Agent và LLM Provider thất bại, dù Admin Web vẫn hiện skill đó như bình thường. Một lần lưu skill bị gián đoạn (ví dụ đầy đĩa) để lại đúng trạng thái gây lỗi khởi động này.
2. **Bản sửa Built-in Job Skill không tới deployment đang chạy.** Server chỉ ghi Built-in Job Skill khi file chưa có. Sau khi nâng cấp, deployment vẫn dùng nội dung cũ cho tới khi Admin bấm khôi phục mặc định từng skill. Admin Web ghi các bản cũ này là "đã sửa", nên Admin không phân biệt được bản mình tự sửa với bản chỉ đơn giản là cũ.
3. **Bỏ gán một skill có thể làm Sub-Agent hỏng âm thầm.** Prompt của một số Sub-Agent yêu cầu nạp các skill cụ thể ngay lượt đầu. Nếu Admin bỏ gán một skill như vậy, Sub-Agent vẫn gọi và bị từ chối, không có cảnh báo nào lúc lưu. Ngoài ra các skill này mất khỏi ngữ cảnh sau khi Agent Session bị nén, và Sub-Agent phải tự nhớ nạp lại.
4. **Skill hướng dẫn gọi tool mà Sub-Agent không có.** Ví dụ `wiki-query` bảo ghi synthesis và vá trang Wiki, nhưng `wiki_researcher` chỉ có tool đọc. Admin gán skill mà không biết Sub-Agent có làm theo được hay không.

## Solution

- Server luôn khởi động được và luôn lưu cấu hình được, bất kể thư mục skill chứa gì. Built-in Job Skill hỏng trên đĩa được tự phục hồi về mặc định. Custom Job Skill hỏng hiện trong Admin Web kèm lý do, và Admin xoá hoặc ghi đè được.
- Sau khi nâng cấp server, Built-in Job Skill mà Admin chưa sửa tự nhận nội dung mới. Skill Admin đã sửa được giữ nguyên và Admin Web ghi rõ "bản mặc định có cập nhật".
- Skill mà một Built-in Sub-Agent không thể thiếu trở thành **Required Job Skill**: nội dung nằm sẵn trong system prompt của Sub-Agent đó, Admin sửa được nội dung nhưng không bỏ gán được.
- Mỗi Job Skill khai báo tool nó cần. Admin Web cảnh báo khi một skill được gán cho Sub-Agent thiếu tool đó. Các Built-in Job Skill được dọn lại để phân bổ mặc định không còn cảnh báo nào.

## Thuật ngữ

Ba khái niệm mới, chưa có trong `GLOSSARY.md`; ticket 13 thêm vào:

- **Required Job Skill**: Built-in Job Skill mà một Built-in Sub-Agent luôn cần; nội dung được server đưa sẵn vào system prompt của Sub-Agent đó và Admin không bỏ gán được. Không phải Loaded Skill (thứ Agent Session tự nạp qua tool `skill`).
- **Skill Tool Requirement**: danh sách tool mà một Job Skill khai báo là cần để làm theo được. Không phải Tool Grant: khai báo này không cấp quyền.
- **Skill Base Version**: bản mặc định mà một Built-in Job Skill trên đĩa xuất phát từ đó; dùng để biết Admin đã sửa hay chưa và bản mặc định có mới hơn hay không.

## User Stories

### Khởi động và cấu hình không bị chặn

1. As an Admin, I want the server to start even when one skill file on disk is malformed, so that a single bad file never takes down Voice and Background Tasks.
2. As an Admin, I want the server to start when the skills directory contains an empty directory or a directory with an invalid name, so that leftovers from an interrupted save or a manual copy do not block startup.
3. As an Admin, I want saving Sub-Agent settings to succeed when an unrelated skill on disk is malformed, so that I can keep configuring the system while I fix the skill.
4. As an Admin, I want saving an LLM Provider to succeed when a skill on disk is malformed, so that provider changes are never blocked by skill content.
5. As an Admin, I want a malformed Built-in Job Skill on disk to be restored to its default automatically, so that core Wiki and Daily workflows keep working.
6. As an Admin, I want the rejected content of a restored Built-in Job Skill kept beside it, so that I can recover my edits.
7. As an Admin, I want a failed skill save to leave no partial directory behind, so that the next restart is not affected.
8. As an Admin, I want the Skill Catalog shown in Admin Web to match exactly what Sub-Agents can load, so that I never assign a skill that the runtime does not have.
9. As an Admin, I want to see a malformed Custom Job Skill in the list with the reason it was rejected, so that I know why Sub-Agents no longer see it.
10. As an Admin, I want to delete a malformed Custom Job Skill from Admin Web, so that I can clean up without shell access.
11. As an Admin, I want to overwrite a malformed Custom Job Skill with valid content from Admin Web, so that I can repair it in place.
12. As an Admin, I want a Sub-Agent that was granted a now-missing skill to keep running without that skill, so that one missing skill does not disable the Sub-Agent.
13. As an Admin, I want a deployment with no skills directory configured to show an empty Skill Catalog consistently, so that list and detail views do not disagree.

### Nâng cấp Built-in Job Skill

14. As an Admin, I want Built-in Job Skills I never edited to receive the new default content after a server upgrade, so that fixes shipped with the server reach my deployment.
15. As an Admin, I want Built-in Job Skills I edited to stay exactly as I left them after an upgrade, so that my customisations are never overwritten.
16. As an Admin, I want an existing deployment that predates this feature to have its untouched Built-in Job Skills upgraded on first start, so that older deployments are not stuck on stale content.
17. As an Admin, I want a skill I restored to default through Admin Web to count as unedited, so that it keeps receiving future default updates.
18. As an Admin, I want to see which of my edited Built-in Job Skills have a newer default available, so that I can decide whether to merge or restore.
19. As an Admin, I want "edited" and "default has an update" shown as separate states, so that I can tell my own changes from stale content.
20. As an Admin, I want a new Built-in Job Skill introduced by an upgrade to appear automatically, so that I do not have to create it by hand.
21. As an Admin, I want running Agent Runs to keep the skill content they started with during an upgrade or a save, so that work in progress is not disturbed.

### Required Job Skill

22. As an Admin, I want the skills a Built-in Sub-Agent cannot work without to be always in its context, so that the Sub-Agent never starts work without its workflow.
23. As an Admin, I want to be unable to unassign a Required Job Skill, so that I cannot break a core workflow by accident.
24. As an Admin, I want to still edit the content of a Required Job Skill, so that I can tune the workflow for my deployment.
25. As an Admin, I want my edit to a Required Job Skill to apply to the next Agent Run without restarting, so that tuning is as quick as today.
26. As an Admin, I want to see which skills are required for each Sub-Agent in Admin Web, so that I understand why they cannot be removed.
27. As an Admin, I want a Sub-Agent with a custom system prompt to still receive its Required Job Skills, so that customising the prompt does not drop the workflow.
28. As an Admin, I want a custom prompt that still tells the Sub-Agent to load a Required Job Skill to keep working, so that prompts saved before this change are not broken.
29. As an Admin, I want Required Job Skills to survive context compaction, so that long Wiki Auto-Flow runs do not lose their rules midway.
30. As an Admin, I want Sub-Agents to stop spending a model turn loading skills they always load, so that each Agent Run is faster and cheaper.
31. As an Admin, I want the system prompt I see and edit in Admin Web to contain only my text, so that the server-owned skill block is not something I can corrupt.

### Skill Tool Requirement

32. As an Admin, I want each Job Skill to state which tools it needs, so that I know which Sub-Agents can follow it.
33. As an Admin, I want a warning when a skill is assigned to a Sub-Agent that lacks a tool the skill needs, so that I can fix the grant or the assignment.
34. As an Admin, I want that warning to not block saving, so that existing settings keep working and I decide when to fix them.
35. As an Admin, I want to declare the tools a Custom Job Skill needs when I create or edit it, so that my own skills get the same check.
36. As an Admin, I want the default assignment of Built-in Job Skills to produce no warnings, so that a fresh deployment is consistent.
37. As an Admin, I want `wiki_researcher` to receive only read guidance from `wiki-query`, so that it is not told to write Wiki pages it cannot write.
38. As an Admin, I want the guidance for filing a synthesis back into the Wiki to live in its own skill assigned to `wiki_curator`, so that the Sub-Agent that can write is the one told to write.
39. As an Admin, I want skill suggestions for a Sub-Agent to exclude skills it cannot follow, so that suggestions are useful.
40. As an Admin, I want a skill's tool list to never grant tools, so that assigning a skill cannot widen what a Sub-Agent may do.

## Implementation Decisions

### Một cách quét Skill Catalog (ADR 0142)

- Thư viện orchestrator-agent công khai một hàm quét thư mục skill ở chế độ khoan dung, trả về các skill hợp lệ và danh sách mục bị bỏ qua kèm lý do. Lúc build `Orchestrator` và lúc `reload_skills` đều dùng hàm này. Chế độ quét nghiêm ngặt không còn được dùng ở đường khởi động.
- Server bỏ phần quét riêng của mình. Thành viên của Skill Catalog (dùng để kiểm tra khi lưu cấu hình, để tính `allowed_skills` hiệu lực và để hiện cho Admin) lấy từ hàm quét của thư viện. Server không còn coi bản nhúng là skill tồn tại khi file trên đĩa hỏng.
- Lúc build và lúc `reconfigure`, một Sub-Agent tham chiếu skill không có trong Skill Catalog không còn là lỗi. Thư viện thu hẹp quyền về các skill đang có và ghi cảnh báo. Thu hẹp luôn an toàn; kiểm tra chính tả ID là việc của server khi Admin lưu cấu hình.
- Một thư mục không có `SKILL.md` bị bỏ qua khi quét, không phải lỗi.
- Server tự phục hồi Built-in Job Skill lúc khởi động và trước mỗi lần ghi skill: file thiếu thì ghi bản mặc định; file không parse được thì chuyển nội dung cũ sang một file ẩn cạnh nó rồi ghi bản mặc định. File ẩn không xuất hiện trong danh sách file liên quan mà Sub-Agent thấy.
- Lần lưu skill thất bại phải dọn thư mục nó vừa tạo.
- Deployment không cấu hình thư mục skill có Skill Catalog rỗng ở mọi route. Riêng Wiki Ask và Required Job Skill vẫn lấy nội dung từ bản nhúng khi không có bản trên đĩa.

### Custom Job Skill hỏng trong Admin API

- Danh sách skill trả thêm các mục bị bỏ qua, với trạng thái `invalid` và lý do. Mục hợp lệ có trạng thái `ok`.
- Xoá một Custom Job Skill `invalid` được phép với ETag tính trên nội dung file thô. Tạo mới bằng `If-None-Match: *` không ghi đè được mục `invalid`; ghi đè dùng `If-Match` với ETag đó.
- Built-in Job Skill không bao giờ ở trạng thái `invalid` vì đã được tự phục hồi.

### Nâng cấp Built-in Job Skill (ADR 0142)

- Server giữ một file manifest ẩn ở gốc thư mục skill, ghi Skill Base Version của từng Built-in Job Skill dưới dạng hash nội dung.
- **Hash nội dung** là SHA-256 của bản render chuẩn hoá của tài liệu đã parse, không phải của byte thô. Nhờ đó bản Admin khôi phục qua Admin Web (được render lại) trùng với bản mặc định.
- Quy tắc cho từng Built-in Job Skill, áp dụng lúc khởi động:

  | Trạng thái trên đĩa | Hành động | Skill Base Version sau đó |
  |---|---|---|
  | Không có file | Ghi bản mặc định | Bản mặc định hiện tại |
  | Không parse được | Giữ nội dung cũ ở file ẩn, ghi bản mặc định | Bản mặc định hiện tại |
  | Trùng bản mặc định hiện tại | Không làm gì | Bản mặc định hiện tại |
  | Trùng Skill Base Version đã ghi, khác bản mặc định hiện tại | Ghi đè bằng bản mặc định | Bản mặc định hiện tại |
  | Chưa có Skill Base Version, trùng một bản mặc định cũ đã biết | Ghi đè bằng bản mặc định | Bản mặc định hiện tại |
  | Chưa có Skill Base Version, không trùng bản nào | Giữ nguyên | Bản mặc định hiện tại |
  | Khác Skill Base Version đã ghi | Giữ nguyên | Không đổi |

- "Bản mặc định cũ đã biết" là một danh sách hash cố định nhúng trong server, sinh một lần từ lịch sử git của các Built-in Job Skill tính tới lúc ticket này hoàn thành. Danh sách này chỉ phục vụ deployment có trước manifest và không cần bổ sung về sau.
- Khi Admin lưu một Built-in Job Skill, Skill Base Version của nó được đặt thành bản mặc định hiện tại.
- Tóm tắt skill trong Admin API thêm cờ "bản mặc định có cập nhật": đúng khi skill đang khác bản mặc định **và** Skill Base Version khác bản mặc định hiện tại. Cờ "đã sửa" hiện có giữ nguyên nghĩa.
- Việc nâng cấp lúc khởi động xảy ra trước khi `Orchestrator` được build, nên không cần hot-reload.

### Required Job Skill (ADR 0143)

- Mỗi Built-in Sub-Agent có thêm danh sách Required Job Skill, cố định trong code:
  - `wiki_ingestor`: `wiki-ingest`, `wiki-taxonomy`, `wiki-page-format`
  - `wiki_curator`: `wiki-lint`, `wiki-query`
  - Các Sub-Agent khác: không có. Skill mà prompt chỉ nạp theo điều kiện vẫn là skill thường.
- Root Orchestrator và Custom Sub-Agent không có Required Job Skill.
- Server ghép nội dung Required Job Skill vào file prompt của Sub-Agent dưới dạng một khối do server sở hữu, đặt sau prompt của Admin hoặc prompt mặc định. Khối này mở đầu bằng câu nói rõ các skill đã được nạp, phải làm theo trừ khi trái với phần prompt phía trên, và không cần gọi `skill` cho chúng. Cách làm theo đúng tiền lệ khối danh sách Sub-Agent trong prompt của Root Orchestrator và tiền lệ `wiki-chat` trong Wiki Ask.
- Prompt lưu trong cấu hình và prompt Admin thấy trong Admin API không chứa khối này. Nếu prompt đã lưu có sẵn một bản chụp của khối, server thay nó khi ghi file.
- Khối được ghi lại mỗi khi lưu skill, lưu cấu hình Sub-Agent và lúc khởi động. Nội dung lấy từ bản trên đĩa nếu hợp lệ, nếu không thì từ bản nhúng.
- Required Job Skill vẫn nằm trong `allowed_skills` hiệu lực của Sub-Agent, nên vẫn nạp được bằng tool `skill`. Lý do: prompt Admin đã tuỳ chỉnh từ trước có thể còn câu yêu cầu nạp. Nếu Admin gửi `allowed_skills` thiếu Required Job Skill, server tự thêm lại chứ không từ chối, giống cách tool `skill` đang được tự thêm.
- Projection cấu hình Sub-Agent trong Admin API thêm danh sách Required Job Skill.
- Prompt mặc định của `wiki_ingestor` và `wiki_curator` (do `llm-wiki-rs` sở hữu và được `orchestrator-flow` tái xuất) bỏ bước nạp các Required Job Skill.
- Không cần thay đổi thư viện orchestrator-agent cho phần này.

### Skill Tool Requirement (ADR 0144)

- Frontmatter của `SKILL.md` thêm trường tuỳ chọn `requires_tools`: danh sách tên tool cách nhau bằng dấu phẩy trên một dòng. Tài liệu skill của thư viện orchestrator-agent mang trường này qua parse và render. Trường rỗng không được render ra, để skill cũ giữ nguyên hash nội dung.
- Tên trong `requires_tools` phải đúng định dạng tên tool; tool không tồn tại trong server không phải lỗi parse (skill có thể được viết cho tool sẽ có sau).
- Khai báo này không cấp quyền và không ảnh hưởng tới việc Sub-Agent có nạp được skill hay không.
- Projection cấu hình Sub-Agent trong Admin API thêm danh sách "khoảng trống tool": mỗi mục gồm ID skill và các tool còn thiếu, tính bằng `requires_tools` của từng skill trong `allowed_skills` hiệu lực trừ đi `allowed_tools` hiệu lực. Đây là cảnh báo; lưu cấu hình không bị từ chối.
- Tóm tắt skill và payload lưu skill trong Admin API thêm `requires_tools`.
- Gợi ý skill cho một Built-in Sub-Agent không còn chứa skill mà Sub-Agent đó không thể có đủ tool.
- Dọn Built-in Job Skill:
  - Tách phần "lưu câu trả lời thành synthesis" khỏi `wiki-query` thành Built-in Job Skill mới `wiki-synthesis-filing`, gán mặc định cho `wiki_curator`. `wiki-query` chỉ còn hướng dẫn đọc.
  - Các Built-in Job Skill khai báo `requires_tools` là tập tool tối thiểu chung cho mọi Sub-Agent được gán mặc định. Phần hướng dẫn dùng tool mà chỉ một số Sub-Agent có phải viết theo dạng có điều kiện.
  - Bỏ `daily-synthesis-staging` khỏi gợi ý của `media_creator` và `media_processor`.

### Thứ tự phát hành thư viện

Theo `AGENTS.md`: làm trong repo thư viện, thử từ server bằng `[patch.crates-io]` tạm, publish theo thứ tự orchestrator-agent rồi orchestrator-flow, thay patch bằng phiên bản đã publish trước khi merge. Bước publish lên crates.io cần người duyệt.

## Testing Decisions

Test tốt chỉ kiểm hành vi nhìn thấy từ bên ngoài: phản hồi Admin API, nội dung file trong thư mục skill và thư mục prompt, và việc app dựng lên được hay không. Không test cấu trúc manifest, tên hàm hay thứ tự gọi nội bộ.

- **Seam chính: Admin API qua HTTP trên một app dựng với thư mục skill và thư mục prompt tạm.** Đây là seam hiện có của test CRUD skill và cấu hình Sub-Agent (`background_skills_crud_etag_preconditions_and_agent_visibility` và các test cùng nhóm). Mọi hành vi trong spec quan sát được ở đây:
  - Chuẩn bị file trên đĩa, dựng app, gọi Admin API, đọc lại file.
  - Nâng cấp Built-in: ghi file cũ lên đĩa trước khi dựng app, rồi kiểm nội dung và cờ sau khi dựng.
  - Required Job Skill: đọc file prompt của Sub-Agent sau khi dựng app, sau khi lưu skill và sau khi lưu cấu hình.
  - Khởi động không bị chặn: dựng app trên thư mục skill có mục hỏng phải thành công.
- **Seam thứ hai: API công khai của thư viện orchestrator-agent**, test trong repo thư viện theo các test quét skill và `reload_skills` hiện có: hàm quét khoan dung, build không lỗi với thư mục có mục hỏng, `reconfigure` thu hẹp skill lạ, `requires_tools` round-trip qua parse và render.
- **Admin Web:** test component theo các test trang Background hiện có, cho huy hiệu "bản mặc định có cập nhật", mục `invalid`, Required Job Skill bị khoá và cảnh báo khoảng trống tool.
- Một test bất biến ở server: với phân bổ mặc định, không Built-in Sub-Agent nào có khoảng trống tool, và mọi Required Job Skill là Built-in Job Skill tồn tại.
- Chạy test bằng `scripts/agent-test.sh`, test có mục tiêu trước rồi mới tới cả workspace.

## Out of Scope

- Required Job Skill cho Custom Sub-Agent hoặc do Admin tự chọn.
- Từ chối lưu cấu hình khi có khoảng trống tool.
- Trộn tự động nội dung Admin đã sửa với bản mặc định mới. Admin tự chọn giữ hoặc khôi phục.
- Lịch sử phiên bản hay hoàn tác cho Job Skill.
- Thay đổi cách Loaded Skill được đưa vào ngữ cảnh qua tool `skill`, và cách nén Agent Session.
- Chuyển Job Skill sang SQLite. `SKILL.md` trên đĩa vẫn là nguồn duy nhất, theo ADR 0128.
- Job Skill của Task Recipe: Task Recipe không dùng Job Skill.

## Further Notes

- ADR 0128 nói việc xoá skill không ghi `agent_settings.json`, nhưng code hiện tại có gỡ ID skill bị xoá khỏi cấu hình đã lưu. Spec này không đổi hành vi đó; ticket 13 sửa lại câu chữ ADR 0128 cho khớp.
- Deployment mà Admin đã ghi đè `allowed_skills` của `wiki_curator` sẽ không tự nhận `wiki-synthesis-filing`. Skill này hiện trong phần gợi ý để Admin tự thêm.
- Bản render chuẩn hoá là cơ sở của hash nội dung. Mọi thay đổi định dạng render về sau phải giữ nguyên kết quả cho tài liệu cũ, nếu không toàn bộ Built-in Job Skill sẽ bị coi là đã sửa.
- Khối Required Job Skill làm system prompt của `wiki_ingestor` dài thêm khoảng 17 KB. Phần này cố định giữa các lượt nên được provider cache.
