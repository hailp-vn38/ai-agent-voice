#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tauri_dir="$repo_root/apps/wiki-desktop/src-tauri"
workspace_dir="$repo_root/apps/wiki-desktop"
sidecar_dir="$tauri_dir/binaries"
sidecar="$sidecar_dir/wiki-mcp-universal-apple-darwin"
out_dir="${WIKI_RELEASE_DIR:-$repo_root/output/wiki-desktop}"
bundle_id="${WIKI_BUNDLE_IDENTIFIER:-com.vela.wikidesktop}"
signing_identity="${APPLE_SIGNING_IDENTITY:-}"
notary_profile="${NOTARY_PROFILE:-}"

fail() {
	echo "release-wiki-desktop-macos: $*" >&2
	exit 1
}

[[ "$(uname -s)" == "Darwin" ]] || fail "macOS is required to build, sign, notarize, and create the DMG."
[[ -n "$signing_identity" ]] || fail "set APPLE_SIGNING_IDENTITY to the confirmed Developer ID Application identity."
[[ -n "$notary_profile" ]] || fail "set NOTARY_PROFILE to a notarytool keychain profile configured by the release owner."
[[ "$bundle_id" =~ ^[A-Za-z0-9.-]+\.[A-Za-z0-9.-]+$ ]] || fail "invalid WIKI_BUNDLE_IDENTIFIER: $bundle_id"

command -v rustup >/dev/null || fail "rustup is required."
command -v cargo >/dev/null || fail "cargo is required."
command -v npm >/dev/null || fail "npm is required."
command -v lipo >/dev/null || fail "Xcode Command Line Tools (lipo) are required."
command -v xcrun >/dev/null || fail "Xcode Command Line Tools (xcrun) are required."
command -v hdiutil >/dev/null || fail "hdiutil is required."

for target in aarch64-apple-darwin x86_64-apple-darwin; do
	rustup target add "$target"
done

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/wiki-release.XXXXXX")"
overlay="$tmp_dir/tauri-release.json"
cleanup() {
	rm -rf "$tmp_dir"
	rm -f "$sidecar"
	rmdir "$sidecar_dir" 2>/dev/null || true
}
trap cleanup EXIT

mkdir -p "$sidecar_dir" "$out_dir"

# Tauri's universal build consumes one sidecar named with the universal target.
# Build both supported slices explicitly, then combine them before bundling.
for target in aarch64-apple-darwin x86_64-apple-darwin; do
	cargo build --locked --release --manifest-path "$repo_root/crates/wiki-mcp/Cargo.toml" \
		--bin wiki-mcp --target "$target"
done
lipo -create \
	"$repo_root/target/aarch64-apple-darwin/release/wiki-mcp" \
	"$repo_root/target/x86_64-apple-darwin/release/wiki-mcp" \
	-output "$sidecar"
chmod 755 "$sidecar"
lipo -verify_arch arm64 x86_64 "$sidecar"

node - "$bundle_id" "$signing_identity" >"$overlay" <<'NODE'
const [identifier, signingIdentity] = process.argv.slice(2);
process.stdout.write(JSON.stringify({
  identifier,
  bundle: { macOS: { signingIdentity } },
}));
NODE

npm run tauri --workspace @vela/wiki-desktop -- build \
	--target universal-apple-darwin --bundles app --config "$overlay"

app="$tauri_dir/target/universal-apple-darwin/release/bundle/macos/Wiki.app"
[[ -d "$app" ]] || fail "Tauri did not produce $app"
app_executable="$app/Contents/MacOS/wiki-desktop"
[[ -x "$app_executable" ]] || fail "Tauri did not bundle executable $app_executable"
lipo -verify_arch arm64 x86_64 "$app_executable"
bundled_sidecar="$app/Contents/MacOS/wiki-mcp"
[[ -x "$bundled_sidecar" ]] || fail "Tauri did not bundle executable $bundled_sidecar"
lipo -verify_arch arm64 x86_64 "$bundled_sidecar"

# Sign the nested command first; then sign the enclosing app without --deep.
codesign --force --options runtime --timestamp --sign "$signing_identity" "$bundled_sidecar"
codesign --force --options runtime --timestamp --entitlements "$tauri_dir/Entitlements.plist" \
	--sign "$signing_identity" "$app"
codesign --verify --strict --verbose=2 "$bundled_sidecar"
codesign --verify --deep --strict --verbose=2 "$app"

version="$(node -e 'process.stdout.write(require(process.argv[1]).version)' "$workspace_dir/package.json")"
app_zip="$tmp_dir/Wiki-$version-universal.zip"
ditto -c -k --sequesterRsrc --keepParent "$app" "$app_zip"
xcrun notarytool submit "$app_zip" --keychain-profile "$notary_profile" --wait
xcrun stapler staple "$app"
xcrun stapler validate "$app"
spctl --assess --type execute --verbose=4 "$app"
spctl --assess --type execute --verbose=4 "$bundled_sidecar"

dmg="$out_dir/Wiki-$version-universal.dmg"
rm -f "$dmg"
hdiutil create -volname Wiki -srcfolder "$app" -format UDZO -ov "$dmg"
codesign --force --timestamp --sign "$signing_identity" "$dmg"
codesign --verify --strict --verbose=2 "$dmg"
xcrun notarytool submit "$dmg" --keychain-profile "$notary_profile" --wait
xcrun stapler staple "$dmg"
xcrun stapler validate "$dmg"
spctl --assess --type open --context context:primary-signature --verbose=4 "$dmg"

echo "Release artifact: $dmg"
