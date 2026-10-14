start:
    (cd packages/native && bun run build:embedded)
    cargo run --release --manifest-path packages/native/Cargo.toml --no-default-features --features lua54,quickjs --bin gpx -- --watch --width 1280 --height 800 examples/luax-workspace/main.luax

start-solid:
    (cd packages/native && bun run build:embedded)
    cargo run --release --manifest-path packages/native/Cargo.toml --no-default-features --features lua54,quickjs --bin gpx -- --watch examples/solid-workspace/main.luax

package-solid:
    bash scripts/package-solid.sh
