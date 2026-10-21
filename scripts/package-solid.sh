#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "This packaging script currently supports macOS only." >&2
    exit 1
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
identity="${MACOS_SIGNING_IDENTITY:--}"
if [[ "${MACOS_NOTARIZE:-0}" == "1" ]]; then
    [[ "$identity" != "-" ]] || { echo "Notarization requires MACOS_SIGNING_IDENTITY" >&2; exit 1; }
    : "${APPLE_ID:?Set APPLE_ID}"
    : "${APPLE_TEAM_ID:?Set APPLE_TEAM_ID}"
    : "${APPLE_APP_SPECIFIC_PASSWORD:?Set APPLE_APP_SPECIFIC_PASSWORD}"
fi
export MACOSX_DEPLOYMENT_TARGET=14.0

mkdir -p dist
stage="$(mktemp -d "$root/dist/.solid-package.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
app="$stage/GPUIX Solid.app"
(cd packages/native && bun run build:embedded)
cargo build --release --manifest-path packages/native/Cargo.toml --no-default-features --features lua54,quickjs --bin gpx
packages/native/target/release/gpx package examples/solid-workspace/gpuix.json --output "$app"

plutil -lint "$app/Contents/Info.plist"
if [[ "$identity" == "-" ]]; then
    codesign --force --sign - "$app"
else
    codesign --force --sign "$identity" --timestamp --options runtime "$app"
fi
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
if [[ "${MACOS_NOTARIZE:-0}" == "1" ]]; then
    ditto -c -k --sequesterRsrc --keepParent "$app" "$stage/notarization.zip"
    xcrun notarytool submit "$stage/notarization.zip" --wait --output-format json \
        --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
        --password "$APPLE_APP_SPECIFIC_PASSWORD" > "$stage/notarization-result.json"
    cat "$stage/notarization-result.json"
    status="$(plutil -extract status raw -o - "$stage/notarization-result.json")"
    [[ "$status" == "Accepted" ]] || { echo "Notarization was not accepted" >&2; exit 1; }
    xcrun stapler staple "$app"
    xcrun stapler validate "$app"
    spctl --assess --type execute --verbose "$app"
fi
ditto -c -k --sequesterRsrc --keepParent "$app" "$stage/GPUIX-Solid-macos-$(uname -m).zip"
rm -rf "$root/dist/GPUIX Solid.app"
mv "$app" "$root/dist/GPUIX Solid.app"
mv "$stage/GPUIX-Solid-macos-$(uname -m).zip" "$root/dist/"
echo "Packaged: $root/dist/GPUIX Solid.app"
echo "Archive: $root/dist/GPUIX-Solid-macos-$(uname -m).zip"
