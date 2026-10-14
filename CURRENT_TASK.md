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
- Zed fork rebased onto upstream c2e9f882030 as branch `gpuix-rebase`, pushed
  to Aeolun/zed (origin is now SSH). `.gitmodules` still names the old
  `gpuix` branch; Bart hasn't chosen between force-pushing `gpuix` and
  pointing `.gitmodules` at `gpuix-rebase`.
- `wip` pushed to serialexp/gpuix (e42c991).
- solid.lua: keyed `create_selector`; text/style/prop bindings skip unchanged
  values. `lua_runtime.rs` keeps a per-window `RootIndex` (handlers, tokens,
  memo aliases, element placements), so slot updates no longer walk the
  tree, and a slot whose new child fails to mount stays unchanged. A
  parent/child swap can't happen in Lua: a slot only takes fresh nodes or
  its own children (`solid_slots_reject_children_mounted_elsewhere`).
  Staged nodes keep their own arena generation and survive another window's
  render while queued updates need them; each event drops the queued
  updates that can never apply. One leftover is in TODO.md (a `solid.For`
  list after a failed window render).

## Verification (macOS)

- Rust lib tests: 367 passed with Lua 5.4, 428 with `--features quickjs`; the Lua runtime tests also pass with LuaJIT. Use a real path
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

1. QuickJS: follow `docs/design/embedded-quickjs-runtime.md`. Phase 0's
   measurements are done (in rust-js-vm; numbers under open question 1 and
   in rust-js-vm's README), and open question 1 is settled: real
   `@gpuix/react`. Phase 1 is done, rescoped: `src/script/app.rs` and
   `reload.rs` (see the doc's 2026-10-13 notes). Phase 2 is done:
   `src/quickjs/` behind the `quickjs` feature, tested from Rust with
   `cargo test --lib --features quickjs --target-dir
   /private/tmp/gpuix-quickjs-target quickjs::`. React updates now get React
   DOM's priorities (a click commits in a microtask). One follow-up is in
   the doc's Outstanding list: the optional `NativeRenderer` methods. Phase 3
   is done: oxc compile (`src/quickjs/compile.rs`), the module loader
   (`modules.rs`, `QuickJsRuntime::load_module`) and the embedded bundles
   (`embedded/build.ts`). The bundles aren't committed: run `bun run
   build:embedded` in `packages/native` before any `--features quickjs`
   build, and again after changing `@gpuix/react` or `@gpuix/native` sources
   (`build.rs` fails with that command when they're stale). Next is Phase 4:
   the `gpx` binary (name picked by Bart on 2026-10-14) that picks the
   engine by file extension and replaces `gpuix-lua`, React parity tests,
   the optional renderer methods, and window options from `render()`.
