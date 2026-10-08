#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "This packaging script currently supports macOS only." >&2
    exit 1
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
export MACOSX_DEPLOYMENT_TARGET=14.0
cargo build --release --manifest-path packages/native/Cargo.toml --no-default-features --features lua54 --bin gpuix-lua

mkdir -p dist
stage="$(mktemp -d "$root/dist/.solid-package.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
app="$stage/GPUIX Solid.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/app"
cp packages/native/target/release/gpuix-lua "$app/Contents/MacOS/GPUIX Solid"
cp -R examples/solid-workspace/. "$app/Contents/Resources/app/"

cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
    <key>CFBundleExecutable</key><string>GPUIX Solid</string>
    <key>CFBundleIdentifier</key><string>dev.gpuix.solid-workspace</string>
    <key>CFBundleName</key><string>GPUIX Solid</string>
    <key>CFBundleDisplayName</key><string>GPUIX Solid</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>0.1.0</string>
    <key>CFBundleVersion</key><string>1</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
</dict></plist>
PLIST

plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - "$app"
codesign --verify --strict "$app"
(cd / && "$app/Contents/MacOS/GPUIX Solid" --check)
otool -L "$app/Contents/MacOS/GPUIX Solid"
while IFS= read -r dependency; do
    case "$dependency" in
        /System/Library/*|/usr/lib/*) ;;
        *)
            echo "Non-system dependency must be bundled before distribution: $dependency" >&2
            exit 1
            ;;
    esac
done < <(otool -L "$app/Contents/MacOS/GPUIX Solid" | tail -n +2 | awk '{print $1}')
ditto -c -k --sequesterRsrc --keepParent "$app" "$stage/GPUIX-Solid-macos-$(uname -m).zip"
rm -rf "$root/dist/GPUIX Solid.app"
mv "$app" "$root/dist/GPUIX Solid.app"
mv "$stage/GPUIX-Solid-macos-$(uname -m).zip" "$root/dist/"
echo "Packaged: $root/dist/GPUIX Solid.app"
echo "Archive: $root/dist/GPUIX-Solid-macos-$(uname -m).zip"
