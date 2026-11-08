# 13: Ký, notarize và đóng DMG

Status: ready-for-human
Blocked by: 12
Spec: ../spec.md (user story 1)

## Mục tiêu

Phát hành bản cài đặt mà macOS cho chạy không cảnh báo.

## Vì sao cần người làm

Cần Apple Developer ID certificate, App Store Connect API key cho `notarytool` và quyết định bundle identifier. Agent không được nhập các credential này.

## Phạm vi

- Build universal (`universal-apple-darwin`). Binary `wiki-mcp` được đóng gói trong bundle và ký cùng app.
- Bật hardened runtime với entitlements tối thiểu.
- Notarize, staple và tạo DMG.
- Script build tái lập được. Agent có thể chuẩn bị script, còn bước nhập credential do người dùng làm.

## Tiêu chí xong

- [ ] `spctl --assess` pass cho app và cho `wiki-mcp`.
- [ ] Cài DMG trên một máy sạch: mở app không có cảnh báo Gatekeeper, Claude Desktop gọi được `wiki-mcp` trong bundle.

## Cập nhật tiến độ

- Đã chuẩn bị script universal arm64/x86_64, Tauri externalBin sidecar, Developer ID/hardened runtime signing, notarytool/stapler, Gatekeeper checks và runbook release owner.
- PASS: kiểm tra shell syntax, JSON/Tauri config, plist và diff.
- NOT_RUN: cần release owner xác nhận bundle identifier, cung cấp Developer ID identity và notarytool Keychain profile; chưa chạy build/sign/notarize, `spctl` hay cài thử trên máy sạch.

## Comments

### Review 2026-11-08 (`/code-review`, diff `3298b791...HEAD`). Các mục còn mở cần sửa trước khi nghiệm thu thủ công.

- [x] Đặt `WIKI_BUNDLE_IDENTIFIER` (script dòng 10) làm đổi thư mục Application Support của app, nhưng `wiki-mcp` hardcode `com.vela.wikidesktop/runtime.json`. Với identifier khác, bridge im lặng ngừng hoạt động. Cần truyền identifier vào sidecar lúc build hoặc bỏ biến này.
- [x] `spctl --assess --type execute` trên binary Mach-O trần (`wiki-mcp`, script dòng 93) thường bị từ chối vì không phải bundle. Kiểm tra lại gate này (vd. `codesign --verify --strict` + kiểm tra notarization ticket) trước khi chạy release thật.

### Sửa sau review (2026-11-08)

- `wiki-mcp` đọc `CFBundleIdentifier` từ `Contents/Info.plist` của app chứa nó, nên runtime file theo đúng identifier của bản release; ngoài bundle thì dùng `com.vela.wikidesktop`. Unit test cho phần đọc plist.
- Script bỏ `spctl` cho sidecar; thay bằng kiểm tra `codesign` có `Authority=Developer ID Application` và cờ hardened runtime. Sidecar được notarize cùng app. Lệch so với tiêu chí "`spctl --assess` pass cho `wiki-mcp`": cần release owner đồng ý.
- NOT_RUN: chưa chạy release thật.
