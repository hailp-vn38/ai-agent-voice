# 03: Độ bền của Task recipe

**What to build:** Task recipe bền như Task thường. Server restart thì Task chạy tiếp từ bước dở. Lỗi tạm thời đưa Task vào Waiting Retry. Hết Job Budget thì Task dừng có kiểm soát. Hủy thì bước đang chạy dừng. Version đã ghim bị gỡ khỏi binary thì Task kết thúc với lý do rõ ràng.

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] Restart giữa bước lặp thì recovery chạy tiếp từ con trỏ bước, không lưu trùng nguồn và không làm lại bước đã commit.
- [ ] Lỗi tạm thời của Tool Step hoặc provider đưa Task vào Waiting Retry, tới `next_attempt_at` thì chạy tiếp từ bước dở.
- [ ] Recipe dùng chung Job Budget. Hết ngân sách thì Task `stopped`, giữ kết quả từng phần.
- [ ] Hủy Task dừng bước đang chạy trong thời gian ân hạn như Task thường.
- [ ] Tool `Mutating` không rõ kết quả không bị chạy lại mù mà đi qua đường ResolveStuckTool có sẵn.
- [ ] Khi resume, nếu version đã ghim không còn trong registry thì Task kết thúc với mã lỗi riêng, không chạy theo version khác.
- [ ] Mỗi hành vi trên có integration test qua interface TaskManager.
