# Phát hành Wiki cho macOS

Script `scripts/release-wiki-desktop-macos.sh` dựng app và sidecar `wiki-mcp` dạng universal (arm64 + x86_64), ký Developer ID với hardened runtime, gửi app ZIP và DMG tới Apple Notary Service, staple ticket, rồi kiểm tra chữ ký và Gatekeeper.

## Chuẩn bị bởi release owner

1. Xác nhận bundle identifier cuối cùng. Mặc định hiện tại là `com.vela.wikidesktop`; có thể truyền quyết định đã duyệt bằng `WIKI_BUNDLE_IDENTIFIER`.
2. Cài Xcode Command Line Tools, Rust targets `aarch64-apple-darwin` và `x86_64-apple-darwin`, Node/npm và các dependency npm của repo.
3. Cài certificate **Developer ID Application** trong Keychain và xác nhận tên identity bằng `security find-identity -v -p codesigning`.
4. Tạo App Store Connect API key có quyền notarization, rồi lưu thông tin vào Keychain profile bằng `xcrun notarytool store-credentials`. Không đưa key, issuer, token hoặc password vào repo hay dòng lệnh release.
5. Chạy trên macOS:

   ```sh
   APPLE_SIGNING_IDENTITY='Developer ID Application: Example, Inc. (TEAMID)' \
   NOTARY_PROFILE='wiki-notary' \
   WIKI_BUNDLE_IDENTIFIER='com.vela.wikidesktop' \
   scripts/release-wiki-desktop-macos.sh
   ```

Script dừng trước khi build nếu thiếu signing identity hoặc notary profile. Có thể đổi thư mục artifact qua `WIKI_RELEASE_DIR`.

## Kết quả và gate phát hành

DMG nằm trong `output/wiki-desktop/`. Script kiểm tra kiến trúc universal, chữ ký của `wiki-mcp` và app, ticket stapled, cùng `spctl --assess` cho app và DMG. `spctl` không đánh giá binary trần nên với `wiki-mcp` script kiểm tra chữ ký Developer ID và hardened runtime; sidecar được notarize cùng app. Release owner vẫn cần cài DMG trên một máy macOS sạch, mở app qua Finder và xác nhận Claude Desktop gọi đúng `Wiki.app/Contents/MacOS/wiki-mcp`; đây là gate thủ công cuối cùng.

Thứ tự notarization dùng `notarytool submit --wait` rồi `stapler staple`/`validate`. Binary con được ký trước app chứa nó; app được ký với hardened runtime và entitlement JIT duy nhất. Sidecar không nhận entitlement; DMG cũng được ký trước khi submit.
