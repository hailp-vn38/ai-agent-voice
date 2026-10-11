# Kết quả kiểm thử Compose test và prod

Cả hai stack đã vượt qua các kiểm tra dưới đây. Gate `bash scripts/compose.sh test run` hoàn tất với exit code **0** sau các sửa lỗi.

| Kiểm tra | Kết quả |
| --- | --- |
| Compose config test và prod | PASS |
| Python tests cho storage checker và Compose wrapper | 12 pass |
| Cargo fmt và Clippy mọi target/feature | PASS |
| Backend mặc định | 602 pass, 0 fail, 4 ignored |
| Backend `qualification-providers` | 605 pass, 0 fail, 4 ignored |
| Web | 126 pass trong 38 file; typecheck và build PASS |
| Compose test smoke trước/sau restart | PASS |
| Prod release build, readiness và cổng web publish | PASS |
| Prod smoke trước/sau restart; SQLite được giữ qua restart | PASS |
| Prod adapter catalog không chứa qualification adapters | PASS |
| Inference VAD Silero, ASR Gipformer, LLM OpenAI, TTS ChillAudio | PASS |

Smoke kiểm tra `/health`, `/ready`, SPA history fallback, Admin auth, proxy WebSocket từ chối kết nối không xác thực, ghi SQLite và đọc lại sau restart. ASR dùng fixture PCM16 mono 16 kHz dài 20 giây. Diagnostic provider đã chạy thành công; image prod cuối cùng được smoke-test lại trước/sau restart.

## Các lỗi đã sửa

- Loader khởi động gọi `.openai()` với `qualification_llm`, gây panic. Đã chọn timeout theo variant giống database loader và thêm regression test tái hiện lỗi trước khi sửa, pass sau khi sửa.
- Sửa các lỗi Clippy ở sanitizer TTS và test module speaker.
- Sửa fixture destructuring/match enum để biên dịch với qualification feature.
- Test materializer dùng file `.dylib` của máy local nằm ngoài Docker context; đã thay bằng fixture byte cố định cho fingerprint, không cần native library.
- Sửa kỳ vọng TTS text đã lọc dấu câu, giữ nguyên assertion text hiển thị.
- Fixture Device admission có hello deadline 500 ms chạy đua với SQLite; đã tái hiện bằng độ trễ 600 ms và xác nhận pass với fixture deadline 5 giây.
- Sửa kỳ vọng catalog/asset ownership cho qualification adapters.
- Smoke dùng Speaker key có dấu `-` không hợp lệ; đã đổi thành `docker_smoke` tại bước tạo và đọc lại.

## Môi trường và dọn dẹp

Docker mặc định lưu tại `/var/lib/docker`, nên storage guard của repo chặn build/up trực tiếp. Kiểm thử dùng daemon Docker-in-Docker tạm với storage trên `/mnt/storage`; không thay cấu hình daemon host.

Đã tạo `docker/config.prod.toml` (git-ignored, quyền 0600) từ cấu hình và credential local có sẵn, bật Runtime Manager với ngân sách cho các adapter triển khai. File này được giữ lại; credential không được đưa vào báo cáo/log đã lưu.

Đã xóa container/network/volume của các stack kiểm tra, daemon tạm, image Docker-in-Docker, toàn bộ image/build cache/Cargo target và bản sao models/database dùng trong lần test. Các container, image và volume có sẵn của dự án khác được giữ nguyên. Giải phóng khoảng **61.0 GiB** trên `/mnt/storage`.

## Giới hạn

8 lượt test opt-in được ignored theo thiết kế; chưa kiểm thử thiết bị voice thật, tải lớn, vision và toàn bộ model/adapter tùy chọn. Docker host mặc định vẫn cần storage nằm dưới `/mnt/storage` để chạy wrapper build/up mà không dùng daemon riêng.

Log cuối: [Compose test](logs/test.log), [prod build](logs/prod-build.log), [prod smoke cuối](logs/prod-final-smoke.log), [provider inference](logs/prod-inference.log), [regression trước sửa](logs/startup-regression-red.log), [regression sau sửa](logs/startup-regression-green.log).
