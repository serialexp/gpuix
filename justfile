start:
    bun scripts/app.ts dev examples/luax-workspace/gpuix.json -- --watch --width 1280 --height 800

start-solid:
    bun scripts/app.ts dev examples/solid-workspace/gpuix.json -- --watch

package-solid:
    bash scripts/package-solid.sh

start-tray:
    bun scripts/app.ts dev examples/tray/gpuix.json

start-tray-js:
    bun scripts/app.ts dev examples/tray/gpuix-js.json

package manifest:
    (cd packages/native && bun run build:embedded)
    cargo run --release --manifest-path packages/native/Cargo.toml --no-default-features --features lua54,quickjs --bin gpx -- package {{quote(manifest)}}
