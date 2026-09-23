# ADR 0046 — Turn identity và writer terminal outcome trên WebSocket sống lâu

## Status

Accepted

> Bổ sung ADR-0013; thay riêng ownership tạo wire `tts:start`/`tts:stop` và terminal outcome trong ADR-0045. Điều kiện AEC, VAD capture và interruption gate của ADR-0045 vẫn áp dụng.

## Context

Nhiều Conversational Turn hoàn tất bình thường có thể cùng Generation ID trên một Voice Session. Nếu LLM operation dùng `(session, generation, generation)`, event muộn của turn trước có thể trùng identity với turn sau. Đồng thời `SpeechOutput::Drained` chỉ đóng producer/pacer phía server; nó không chứng minh normal `tts:stop` đã được WebSocket writer gửi, và actor không thể tự phân xử race giữa normal completion với abort. Xem thêm ADR-0013 về Single Writer và ADR-0045 về Acoustic Barge-in.

## Decision

- Cấp Turn ID duy nhất, tăng đơn điệu, sau utterance terminal boundary và sau Active Turn admission nhưng trước ASR finalization. ASR stream identity và VAD Capture Cycle ID vẫn độc lập; Generation ID tiếp tục là cancellation/invalidation epoch và có thể bao trùm nhiều normal turn. Overflow của Turn ID hoặc Generation ID fail closed.
- Writer là owner duy nhất của wire `tts:start`/`tts:stop`. Actor gửi `BeginTurn`, `Audio`, `FinishTurn` và urgent `AbortTurn`; writer giữ playback state theo Turn ID, gửi tối đa một start/stop và trả `TurnClosed(Normal|Aborted)` hoặc failure. Writer terminal serialization quyết định winner của Finish/Abort race; stop send đã bắt đầu không bị preempt. Terminal lifecycle commands không đi qua generic GenerationGate.
- `SpeechOutput::Drained` chỉ kết thúc production. Actor giữ PendingDelivery và commit Delivered Assistant Response chỉ khi `TurnClosed(Normal)` xác nhận normal stop đã gửi thành công. Đây là server-side WebSocket delivery, không phải client playback completion.
- Mỗi Voice Session có tối đa một Delivery Turn A và một History-Barrier Turn B. B là Active Turn có permit riêng; ASR B có thể hoàn tất trước writer outcome A nhưng user text B chỉ được commit và LLM B chỉ được start sau outcome A. B admission cần global capacity và barrier slot trống; không tạo queue turn không giới hạn.
- V1 `abort` là session-scoped: revoke mọi conversational/capture work chưa terminal và remove B ngay, nhưng A giữ permit tới writer terminal outcome hoặc teardown. Nếu writer đã đóng A Normal, A vẫn Normal dù actor chưa nhận ACK; B vẫn bị abort. Một inbound abort chỉ advance generation một lần.

## Consequences

`ActiveTurnPermit` phải thuộc từng TurnContext thay cho một boolean trong SessionActor. Normal stop admission/send failure không commit assistant và làm Voice Session fail closed/teardown. History-Barrier Turn giữ tối đa một final text có giới hạn 4.096 Unicode scalar; empty, failed hoặc oversize terminalizes B mà không commit user. Test phải bao phủ late worker event, writer Finish/Abort race, start chưa gửi, hai permit A/B, history ordering và session-scoped abort.
