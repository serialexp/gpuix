# Current task

Bring the rust-js-vm experiments into GPUIX: keep the embedded Lua runtime on
top of upstream, then add a QuickJS engine (React-style and Solid-style, with
built-in JSX/TSX compilation) behind a shared core.

## Done

- `wip` rebased onto upstream `origin/main` (6a5c7c1). The snapshot transport
  experiment was dropped entirely. Intermediate rebased commits do not build;
  the final tree does.
- Lua runtime ported to upstream's retained tree API (cold `extras`,
  `focusable_ids`, `motion_ids`, `dirty`); `update_element` keeps them in step
  and has unit tests for state and invalidation.
- Lua array checks use `dense_len`, which behaves the same on Lua 5.4 and LuaJIT.
- Lua-only focus behavior (Enter/Space click, descendant press focusing its
  ancestor, background press refocusing the root) is gated behind
  `BuildCtx.native_focus`. Ungated, it broke upstream's Combobox in React.
- `gpuix.icon` completed from the September stash (build.rs packer,
  `mod phosphor`, `_phosphor_icon`, `phosphor:` svg sources). The stash itself
  is untouched at `stash@{0}`.
- Zed fork rebased locally onto upstream c2e9f882030 as branch `gpuix-rebase`
  (not pushed).

## Verification (macOS)

- Rust lib tests: 353 passed with Lua 5.4 and with LuaJIT. Use a real path
  for `--target-dir` (`/private/tmp/...`, not `/tmp/...`); the updater tests
  reject a symlinked executable path.
- React 538, Solid 19, native Vitest 23, examples 55, CLI pass;
  example-app typecheck and package builds pass.
- `build:web` passes (nightly with `rust-src`, `wasm-bindgen-cli` 0.2.127).
  `/opt/homebrew/bin/cargo` comes first in PATH on this machine and does not
  understand `+nightly`, so run it as `PATH="$HOME/.cargo/bin:$PATH" bun run
  build:web`. The remaining wasm warnings are upstream's.
- The Linux headless renderer was not verifiable on macOS.

## Next

1. Push `wip` (force, after the rebase) and the zed `gpuix-rebase` branch to
   Aeolun/zed, each only when asked.
2. Port the rust-js-vm improvements: keyed `create_selector` in solid.lua;
   skip unchanged values in `solid.text`, `bind_style`, `bind_prop`; check the
   parent/child swap in one update; the full-tree-walk TODO.
3. QuickJS: follow `docs/design/embedded-quickjs-runtime.md`. Phase 0's
   measurements are done (in rust-js-vm; numbers under open question 1 and
   in rust-js-vm's README), and open question 1 is settled: real
   `@gpuix/react`. Next is Phase 1, the shared `src/script/` core.
