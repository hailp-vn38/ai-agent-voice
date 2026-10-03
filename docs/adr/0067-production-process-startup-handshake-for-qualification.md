# ADR 0067 — Production-process startup handshake cho qualification

## Status

Accepted

Mandatory Qualification phải spawn production binary thật và chứng minh restart qua process boundary; không parse human log, reserve trước TCP port hoặc rebuild `AppState` trong harness. `VOICE_AGENT_BOUND_ADDRESS_FILE` và `VOICE_AGENT_STARTUP_NONCE` là atomic pair: cùng absent giữ startup bình thường, còn missing counterpart hoặc invalid lowercase canonical UUID/path fail trước bind. Khi bật handshake, server bind `127.0.0.1:0`, exclusive-create temporary sibling, write/flush rồi atomically rename artifact versioned gồm nonce, PID và bound address tới destination chưa tồn tại trước khi serve. Publish failure đóng listener và fail startup; server không overwrite/xóa artifact. Harness xóa stale artifact trước spawn, chỉ nhận version hỗ trợ/nonce khớp/address hợp lệ từ child nó sở hữu, coi PID là diagnostic/cross-check, rồi bounded-poll `/ready`; artifact xuất hiện không đồng nghĩa Ready.

Controlled restart V1 là Unix-only: gửi SIGTERM, yêu cầu process exit trong deadline rồi spawn child mới với nonce/address mới và cùng database path. Linux là CI authority; non-Unix trả `UNAVAILABLE` trước spawn. Force-kill chỉ được dùng để teardown sau failure, không phải bằng chứng shutdown thành công. Scenario identity là run/spec/resource identity; process identity là harness-owned child + nonce + handshake; network endpoint chỉ là observation tạm thời của từng process lifetime và không được persist làm authority.

Pattern này chọn một machine-readable production-process contract thay cho log parsing dễ drift, port-reservation có race hoặc in-process seam không chứng minh restart barrier. Artifact chỉ là startup discovery, không phải health/readiness endpoint hay credential channel.
