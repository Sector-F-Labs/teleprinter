#!/usr/bin/env bash
# Builds Teleprinter.app as a universal (Apple Silicon + Intel) binary and wraps it in a
# drag-to-Applications .dmg, as Photograph does. Runs on macOS, by hand: `make dmg`. The app is
# ad-hoc signed only, not notarised, so Gatekeeper asks to allow it on first launch.
#
# Output: target/dmg/Teleprinter-<version>.dmg
set -euo pipefail

cd "$(dirname "$0")/../.."
version="$(awk -F\" '/^version = / { print $2; exit }' Cargo.toml)"
out=target/dmg
app="$out/Teleprinter.app"
targets=(aarch64-apple-darwin x86_64-apple-darwin)

for t in "${targets[@]}"; do
  rustup target add "$t" >/dev/null
  cargo build --release --locked --bin teleprinter --target "$t"
done

rm -rf "$out"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create -output "$app/Contents/MacOS/teleprinter" \
  target/aarch64-apple-darwin/release/teleprinter target/x86_64-apple-darwin/release/teleprinter
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"

# The icon: the SVG rendered at every size an .icns holds (with the repo's own renderer, so
# there is nothing to install).
iconset="$out/teleprinter.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  cargo run -q --release --example render_icon -- "$size" "$iconset/icon_${size}x${size}.png"
  cargo run -q --release --example render_icon -- $((size * 2)) "$iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/teleprinter.icns"
rm -rf "$iconset"

# Apple Silicon refuses to run unsigned code; an ad-hoc signature is enough to launch.
codesign --force --deep --sign - "$app"

# The disk image: the app beside an Applications shortcut to drag it onto.
stage="$out/stage"
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
hdiutil create -volname "Teleprinter $version" -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$out/Teleprinter-$version.dmg"
rm -rf "$stage"
echo "Built $out/Teleprinter-$version.dmg"
