start:
    cargo run --release --manifest-path packages/native/Cargo.toml --no-default-features --features lua54 --bin gpuix-lua -- --watch --width 1280 --height 800 examples/luax-workspace/main.luax

start-solid:
    cargo run --release --manifest-path packages/native/Cargo.toml --no-default-features --features lua54 --bin gpuix-lua -- --watch examples/solid-workspace/main.luax

package-solid:
    bash scripts/package-solid.sh
