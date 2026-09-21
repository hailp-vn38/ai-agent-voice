# ADR 0019 — Max utterance kết thúc auto nhưng loại bỏ manual

## Status
Accepted

Khi đạt max utterance ở auto/VAD mode, server force-endpoint utterance, dừng collector và chuyển sang Processing; không thu turn mới song song. Ở manual mode, server discard buffer, đánh dấu capture overflow, bỏ audio tiếp theo đến `listen:stop`, và yêu cầu `listen:start` mới để thu lại. Chính sách này giữ mỗi Voice Session chỉ có một Conversational Turn active.
