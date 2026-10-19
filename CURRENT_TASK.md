# Current task

Bring the rust-js-vm experiments into GPUIX: keep the embedded Lua runtime on
top of upstream, then add a QuickJS engine (React-style and Solid-style, with
built-in JSX/TSX compilation) behind a shared core. rust-js-vm itself is
gone (2026-10-19); its measurements are in
`docs/design/quickjs-phase0-results.md`.

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
- Zed fork rebased onto upstream c2e9f882030 and force-pushed over the
  `gpuix` branch of Aeolun/zed (02535d0, decided by Bart on 2026-10-16), so
  `.gitmodules`' `branch = gpuix` matches the pinned commit again. The
  temporary `gpuix-rebase` branch is deleted.
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

- Rust lib tests: 369 passed with Lua 5.4, 444 with `--features quickjs` (plus 5 `gpx` bin tests); the Lua runtime tests also pass with LuaJIT. Use a real path
  for `--target-dir` (`/private/tmp/...`, not `/tmp/...`); the updater tests
  reject a symlinked executable path.
- React 540, Solid 19, native Vitest 23, examples 55, CLI pass;
  example-app typecheck and package builds pass.
- 2026-10-16, after the React Fast Refresh commit: Rust lib tests 497 with
  `--features quickjs`; React 546 (vitest) and its typecheck, Solid 36,
  native 23. A release `gpx --watch` hot-updated a React app live.
- `build:web` passes (nightly with `rust-src`, `wasm-bindgen-cli` 0.2.127).
  `/opt/homebrew/bin/cargo` comes first in PATH on this machine and does not
  understand `+nightly`, so run it as `PATH="$HOME/.cargo/bin:$PATH" bun run
  build:web`. The remaining wasm warnings are upstream's.
- The Linux headless renderer was not verifiable on macOS.

## Next

The QuickJS port follows `docs/design/embedded-quickjs-runtime.md`; its
checklist is the source of truth for what's done. As of 2026-10-16,
Phases 0 to 5 are in, and Phase 6's React Fast Refresh is too (51c5012).
Nothing since `wip`'s last push is pushed. Another agent has uncommitted
work in `examples/*`, `bun.lock` and `.github/workflows/ci.yml`; leave it
alone.

Run `bun run build:embedded` in `packages/native` before a `--features
quickjs` build, and `bun run build:quickjs` before the JS suites' QuickJS
tests. React's suite runs with `bun run test` (vitest); `bun test` hangs
there. Agent-tool review subagents kept stalling; `claude -p` with the
prompt on stdin worked.

### 1. Solid hot refresh under `gpx --watch` (up next)

Bart decided on 2026-10-16: an upstream PR to solidjs/solid that makes the
compiler's refresh pass usable without napi, and `gpx` pinned to a fork's
revision until it lands. Pushing the fork and opening the PR are
outward-facing: show Bart the diff first.

What's there:
- `solidjs-compiler` is a git dependency pinned to rev `4937f0a2` (the
  `solid-js@2.0.0-rc.15` tag), with `default-features = false`
  (`packages/native/Cargo.toml`).
- `packages/compiler/src/refresh/` (`mod.rs`, `transform.rs`,
  `signature.rs`) holds `transform_refresh(code, options)`. In `lib.rs`
  it sits behind `#[cfg(feature = "node")]`, which pulls in napi.
- Only `mod.rs` uses napi: `#[napi(object)]` on `TransformRefreshOptions`,
  napi's `Result`/`Error::from_reason`, and `crate::config`'s
  `TransformResult` and `source_type_for_filename` (both also node-only).
- Options:
  - `bundler` (`"standard"` or `"vite"`);
  - `fix_render`, which wraps `render()` imported from `@solidjs/web`, so
    it does nothing for `@gpuix/solid`'s `render`;
  - `granular`;
  - `import_source` (use `"solid-js/refresh"`);
  - `filename`;
  - `source_map`.
- The runtime, `solid-js/refresh` (`dist/refresh.dev.js`), uses
  `hot.data`, `hot.accept`, `hot.dispose`, `hot.decline` and
  `hot.invalidate`, plus Solid's `$DEVCOMP`, which only the dev build has.
  Vite's 2.0 plugin runs `transformRefresh` before the JSX compile.
- Unlike React's refresh, the edited component remounts, losing its own
  signals; parents and siblings keep their state.

Plan:
1. The upstream change, in a local clone of solidjs/solid at the pinned
   rev:
   - split the refresh pass from napi: a plain Rust
     `transform_refresh(code, &RefreshOptions) -> Result<RefreshOutput,
     CompileError>`, under no feature or a napi-free `refresh` feature;
   - keep the napi wrapper in `node_adapter`/`refresh` under `node`;
   - make the pieces of `config` it needs napi-free too;
   - add a Rust test in the crate.
   
   Keep the diff small and in upstream's style, so it can become the PR.
2. Pin `gpx` to it: first a `path` dependency on the local clone, for
   development; then, once Bart OKs pushing a fork, a `git` dependency on
   the fork's rev, with a Cargo.toml comment pointing at the PR.
3. Embed `solid-js/refresh` in the development set
   (`packages/native/embedded/build.ts`, like `react-refresh/runtime`), and
   check its `$DEVCOMP` use against the dev client build gpx embeds.
4. Compile: in `Mode::Development`, run `transform_refresh` (import source
   `solid-js/refresh`, `fix_render: false`, `granular: true`) on Solid
   files before `compile_solid`, and set `Compiled.refresh` for them.
   Positions need composing: refresh output, then Solid's map, then the
   source (see `Origin::Solid` in `compile.rs`).
5. Runtime (`src/quickjs/hot.rs`):
   - give each Solid module an `import.meta.hot` with a `data` object kept
     across that file's versions (by unversioned path), and record its
     `accept`, `dispose`, `decline` and `invalidate` calls;
   - pick the bundler mode that reads `import.meta.hot` (`"vite"`), or
     make `"standard"` find it;
   - a Solid update reuses the same planning (`spread`, versions,
     `pending`), but a Solid module is a boundary when it called
     `hot.accept()`, not by `isLikelyComponentType`;
   - run `dispose` callbacks before the new version loads, and treat
     `decline`/`invalidate` as a restart or a widen;
   - there's no `performReactRefresh` step: Solid's runtime swaps the
     components when the new module runs.
   
   `Graph.loaded`'s refresh flag probably becomes a per-framework kind.
6. Tests (in `src/quickjs/tests.rs`, next to the React hot tests):
   - a component swaps and its parent's signal keeps its value;
   - a plain `.ts` change propagates to a component module;
   - an entry change restarts;
   - a compile error leaves the app running.
   
   Then replace `a_solid_app_restarts_on_a_change` and
   `a_solid_apps_plain_typescript_change_restarts_before_it_runs`, check
   live with a release `gpx --watch examples/solid/...`, and run the
   Solid suite.
7. Docs:
   - README's `--watch` paragraph;
   - a changeset;
   - the design doc's Phase 6 Solid item (tick it and add a dated note);
   - this file.

   Then show Bart the upstream diff, and ask before pushing the fork or
   opening the PR (its description ends with the attribution line from
   his CLAUDE.md).

### 2. Rest of Phase 6

- A compile cache by content hash: the design doc's deferred Phase 3 item.
- Benchmark numbers in `docs/lua-runtime.md`'s benchmark table.

### 3. Other follow-ups

These are in TODO.md:
- `--watch` doesn't watch `tsconfig.json`, files outside the entry's
  directory, or non-JS text imports;
- stack traces show `?hot=N` after a hot update;
- report Bun's code-splitting cycle bug;
- clean up the older clippy and fmt drift.

The design doc's Outstanding list has the rest (parity tests in CI, the
optional renderer methods, and others).
