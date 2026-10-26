# Publishing Linux compatibility changes (2026-10-25)

- Bart authorized commit and push. Published rebased Zed commit `03e9509da6af0871ba1bbdc3d0e1266409c08552` to `Aeolun/zed` branch `gpuix-linux-upstream-20261025`, using a normal new-branch push (no remote history rewritten). Updated `.gitmodules` branch to match; URL remains Aeolun/zed.
- GPUIX Linux fixes, upstream integration, tests, CI and documentation are bundled into one whole-file commit on `wip`. Verification results below apply to this code; final diff whitespace check passed. Original local branches remain preserved.

# Linux verification after upstream rebase (2026-10-25)

- Integrated rebased Zed HEAD `03e9509da6af0871ba1bbdc3d0e1266409c08552`, based on upstream `a5c4ca8bf6c4717a328a4a61b42d6d1d38949bcc`. Main build checkout uses named branch `gpuix-linux-final`; original `gpuix` and local preserved branch are intact. Worktree `/home/bart/Projects/zed-gpuix-linux-fonts` holds rebase branch/history. Nothing pushed.
- Root Rust pin updated to 1.98.1; native Cargo.lock follows upstream WGPU30/git and other dependency changes. Adapted GPUIX blur(cx), dynamic keystroke parsing, static input bindings, ScrollEvent/source.
- Linux final tests on rebased GPUI: 524 Rust lib +21 gpx passes, 92 LuaJIT Lua passes; React 546 passes/1 existing skip; Solid 37 passes/no skips (QuickJS parity included); native JS/Lua/package 27 passes; examples 56 passes; starter app 6 passes. Native debug addon + release QuickJS addon + launcher builds pass. All-target cargo check with QuickJS passes. Browser Wasm release build passes. TS builds/typechecks pass; both workflow schemas validate.
- Fork worker verification: GPUI 418 passes/2 ignored; WGPU 48 passes including real headless GPU captures; Linux platform and browser crate checks pass. Human README review marker retained inside fork.
- Fixed Linux QuickJS app loop prematurely exiting on closed unsupported tray channel; regression covers event/reload processing. Fixed Bun/Vitest Zod externalization config + consumer import regression. Fixed Solid clean build source preload + regression. Added headless test file-picker response API + cancellation/selection/error tests; generated N-API types updated. Font registration fixed by upstream cache/generation implementation. Linux JS tests now in CI matrix with default test-support addon. README documents prerequisites and prompt simulation.
- Rebased isolated native packaging smoke passes (Lua/JS, replacement, failure preservation, source-free checks). Debian package generated and metadata inspected at `/tmp/gpuix-linux-rebased.deb` (debug runner); NOT installed. Missing global Node/npm handled with temporary real Node24/npm11 PATH for package tests.
- Scope limitations: no live desktop/X11 interaction smoke, package installation, macOS/Windows compile/execution or CI run performed. macOS embedding port cannot be validated on this Linux host; worker preserved APIs and ported cocoa to objc2. No main GPUiX commit made yet. Fork commit is LOCAL ONLY, so do not push a parent submodule pointer until fork branch is published with Bart approval.

# Linux compatibility and upstream GPUI rebase (2026-10-25, in progress)

- Bart requested making remote work on Linux, then explicitly chose rebasing fork changes on latest upstream rather than porting one font fix. Authorized separate worktree + fetch + rebase; NO push.
- Rebase worker `01a17251-5845-7112-8fbf-2ea00ac06193` owns `/home/bart/Projects/zed-gpuix-linux-fonts`, branch `gpuix-linux-fonts`, rebasing 58 commits from old base `1d7e5f1d014acf0d288c6d11be93c31fe647e725` onto upstream `a5c4ca8bf6c4717a328a4a61b42d6d1d38949bcc` (2026-10-25). Main zed build checkout remains old `gpuix` at `02535d0`. Do not touch worker conflicts. Earlier attempted cherry-pick 9e6e14160c was aborted with Bart permission.
- Parent changes: QuickJS optional tray receiver no longer exits app on unsupported Linux backend; regression covers event + reload. Shared Vitest config transforms Zod to avoid Bun/Vite externalization named-export bug. Solid source preload removes clean-build dist dependency; clean-build regression passes. CI now includes headless Linux JS test matrix/default test-support addon. README adds Linux dependency/test steps.
- Baseline old-GPUI verification after fixes: 524 Rust lib +21 gpx tests pass; LuaJIT Lua suite 92 passes; native JS/Lua 27 passes with real Node/npm PATH + generated web assets; examples 56 passes; example-app 6 passes; Solid with release QuickJS addon 37 passes, no skips; React QuickJS parity 6 passes. React full suite before QuickJS addon: 536 pass/3 fail/7 skip (2 fonts, 1 cancellation timeout).
- Font failure root cause: Cosmic family cache retains missing font, outer GPUI/layout caches also need invalidation. Upstream commit `9e6e14160c` includes complete fix, now should arrive via full rebase.
- Prompt timeout: Linux HeadlessAppContext uses TestPlatform queue, unlike macOS native prompt cancellation. Worker asked to add headless simulate_path_prompt_response/did_prompt_for_paths API; parent must expose deterministic test-addon API/wrapper and adjust tests to explicitly answer queued prompts.
- Dependencies: wasm-bindgen-cli 0.2.127 installed via cargo; native web assets build passed. Real Node 24 binary at `/tmp/bunx-1000-node-linux-x64@24/node_modules/node-linux-x64/bin/node`, npm at `/tmp/bunx-1000-npm@11/node_modules/.bin/npm`. Use both dirs on PATH for native package tests. Node/npm not globally installed.
- Release QuickJS addon build passed; generated index.d.ts may change when adding prompt API. Parent added N-API `respond_to_path_prompt` + JS wrapper + cancellation/selection/error tests, pending worker GPUI API so not built yet. Solid full release-addon suite 37/37, React parity 6/6. Debian packaging smoke created `/tmp/gpuix-linux-verification.deb` and inspected control/contents (debug runner, NOT installed). Isolated native packager smoke passed. Workflow YAML parses; actionlint not installed (npm package has no executable).
- Worker reports all 58 commits replayed; Linux platform cargo check passes, compiling focused tests and requested headless prompt APIs before final commit. Upstream Rust 1.98.1 installed. Parent awaiting worker final to integrate (no main Zed modifications yet).
- Parent public prompt API edits have not compiled yet (headless context methods only in worker). Regenerate index.d.ts via napi build after integration; reviewer flagged missing declaration until then. Both workflow schemas validate via @action-validator/cli, git diff --check and TS builds pass.
- Need final rebuild/retests after rebase integration, match rust-toolchain, update Cargo deps as needed, run package smoke + full suites. No source commits in main GPUiX yet. All original branches preserved.

# Remote comparison and Linux verification (2026-10-25)

- Preserved original local wip at `wip-local-6033b90`; current `wip` tracks `origin/wip` at `de456a9`. Preserved its Zed commit as submodule branch `wip-local-71b66c9`; build checkout is now `gpuix` at remote-required `02535d0`. No merge/reset or source fixes made.
- Compared the nine local commits: equivalents exist remotely, replayed and expanded (Lua/LuaX, Lua Tab navigation, headless Linux renderer, combobox blur dismissal, persistent keyboard focus indicators). Unified `gpx` supersedes `gpuix-lua`; React combobox now relies on blur rather than a direct Tab branch. Remote adds Solid, QuickJS, packaging, devtools and more.
- `bun install --frozen-lockfile`, native debug addon build, native JS and React TypeScript builds passed. Solid build initially cannot resolve its own dist preload on fresh checkout; bootstrap with `bun node_modules/typescript/bin/tsc -p packages/solid/tsconfig.json` then normal Solid build passes.
- Rust default library suite exits 0; 389 tests listed. Solid: 29 pass, 7 QuickJS parity skips (standard addon lacks QuickJS). Native JS/Lua: 25 pass, 2 package failures (npm executable absent; missing generated web Wasm assets).
- React: 344 pass, 2 font-registration tests fail (square-font width differs by 34px), 10 suites cannot load due to Zod `z.enum` import being undefined under Vitest/Bun. All 5 example suites hit the same Zod loading failure. Native Lua focus (6 tests) and icon (2 tests) pass on headless Linux.
- Remaining investigation: distinguish Bun/Vitest Zod import integration issue from dependency packaging; debug Linux registered-font shaping; make clean Solid build bootstrap reliable; install/build appropriate packaging prerequisites before rerunning package checks. These are findings, not fixed in the comparison task. Full React log: `/tmp/gothab-stdio-home-1792918302040/tool-outputs/bg-v3famnsb.log`. No live desktop app was launched.

# Current task

## 2026-10-22: Non-activating embedded app checks

- Added `gpx --background` for Lua/LuaX and QuickJS window creation. Live
  automation and screenshots keep working without activating the application.
  Explicit application activation requests remain allowed; Linux currently
  ignores GPUI's focus option. This is not a headless launcher.
- Agent-run live checks must use this flag; windowless component tests remain
  preferred when they cover the behavior under test.
- Validation: 535 Rust library tests and 21 launcher tests pass. Release gpx
  rebuilt. Live Lua and QuickJS startup, click/state updates and GPU screenshots
  each preserved the desktop foreground application's PID.

## 2026-10-22: Native in-app inspector (supersedes browser-first UX)

- Bart clarified the intended UI: right-click Inspect element and a native
  bottom dock inside the app. The browser page remains an optional external
  companion, not the primary inspector.
- Watch-mode Lua/LuaX and QuickJS apps enable the native inspector without an
  HTTP server/token. Existing `just start` / `just start-solid` use watch mode.
- Dock selection/expansion belongs to each GpuixView. GPUI's anchored/deferred
  popup, uniform_list and scroll handle provide the menu and virtualized tree.
  Native picking consumes the press/release rather than triggering the app.
- Direct retained-node property export skips descendants. Inspector geometry
  reads one window-scoped box; native selection pins the existing paint overlay.
- Layout metrics are public theme overrides, not fixed native layout constants.
- Release Rust suite: 527 passed, including native dock layout, view-lease-safe
  context-menu dispatch and selection/ancestor tests. Release launcher and addon
  rebuilt. React: 540 passed, 6 QuickJS-addon tests skipped; TypeScript built;
  all 55 example tests passed.
- Live Lua checks passed for right-click Inspect, native bottom layout, picking
  without activating controls, keyboard close, unchanged app state and normal
  input afterward. The optional external inspector smoke also still passes.
- Prior external-inspector evidence follows below.

## 2026-10-22: Read-only embedded inspector

- `gpx --dev-server PORT` now prints a bundled `/inspector` URL. Enter the
  existing dev token in the browser; it is not embedded in HTML or persisted.
- The inspector reuses revision-polled detailed host trees, and adds separate
  window-scoped `geometry`, `pick` and temporary `highlight` commands. Paint
  order and GPUI content masks drive picking; native outlines are deferred
  above popups without adding hitboxes or changing layout.
- Accepted HTTP connections now use timed blocking reads. A split-packet test
  covers the intermittent macOS `WouldBlock` errors exposed by browser polling.
- Window discovery, expandable trees, declared props/styles/events, painted
  bounds, manual screenshot previews and preview picking are implemented.
  Framework component ownership, source mapping and live editing remain future
  work. Standalone Bun/Node apps do not expose this embedded server.
- Native server/bounds tests: 14 passed. Browser smoke and a live Lua counter
  check passed, including native outline pixels, screenshot picking, app-input
  updates, highlight expiry and foreign-origin rejection. The release addon and
  launcher were rebuilt. React's final run had 538 passes, 6 QuickJS-addon skips
  (standard addon features) and 2 subprocess timeouts under machine load; both
  failures passed in a serial 87-test events/render rerun. React build passed;
  all 55 example tests passed against the rebuilt addon. Logs/screenshots are under
  `/tmp/gpuix-inspector-*`.

## 2026-10-21: Self-contained native packaging (supersedes source-build prototype)

- Replaced the `gpx package` Bun/checkout wrapper with native Rust packaging.
  The installed executable is the runner template; no source checkout, Bun,
  Cargo or compiler is consulted when packaging an application.
- Versioned archive trailers support ELF, PE32+ and thin Mach-O. Mach-O archives
  extend __LINKEDIT and are located before the code signature. Repackaging strips
  the previous archive; original runners and failed outputs remain untouched.
- Native `.app` and `.deb` builders retain identity, embedded default icons,
  signing and dependency generation. Platform format tools are still required.
- Removed compile-time application embedding, the extra gpx-app target and
  TypeScript packaging/Debian implementations. The Bun script is now dev-only.
- Local isolated-runner smoke tests pass for Lua, React/TypeScript, replacement,
  failure preservation and signed app bundles. Desktop CI runs the same checks
  on each target OS, with Bun/Cargo absent from the packer's PATH.
- Native tests cover ELF replacement, PE certificate removal and Mach-O signature
  boundaries. The same Rust archive/packer/Debian modules passed tests and a real
  apt install/check/remove cycle in a Debian arm64 container via a minimal runner
  harness (no GPUI/Lua execution in that harness). Full Linux/Windows application
  and GUI validation still await their native CI runners.
- Final macOS validation: 20 native tests, development-manifest tests, TypeScript
  checks and actionlint pass. The isolated CLI smoke covers a read-only installed
  runner, binary assets, safe directory/output handling, Lua and React entries,
  replacement without archive growth, and source-free startup. Re-signing a
  generated app with hardened-runtime options preserves its archive and startup.

## 2026-10-21: Packaging belongs to the gpx CLI

- Added `gpx package gpuix.json --format app|binary|deb --output PATH`.
  Packaging recipes and distribution CI now invoke it, rather than exposing
  `scripts/app.ts` as the user-facing package command.
- The CLI currently orchestrates the existing Bun helpers and Cargo builds;
  it still requires a source checkout and build toolchain. Checkout discovery
  supports `GPUIX_SOURCE_DIR`; output defaults to the caller's `dist` directory.
- Added an internal `gpx-app` runner target so Cargo does not overwrite the
  executing `gpx` tool (notably a locked executable on Windows).
- Validation: both launcher targets' tests pass, as do the seven TypeScript
  packaging tests, TypeScript checking and actionlint. A real `gpx package`
  invocation from outside the checkout built a standalone Solid app into that
  caller's `dist`, and the output passed `--check` without source sidecars.

## 2026-10-21: Debian application packages

- Added `--format deb` on native Debian/Ubuntu hosts and optional `linux`
  manifest metadata for package name, maintainer, description, SVG icon and
  desktop categories. Package payloads remain embedded in a native executable.
- Packages install `/usr/bin/<packageName>`, a desktop launcher and a hicolor
  SVG icon. `dpkg-shlibdeps` derives linked dependencies; GPUI runtime-loaded
  libraries are added explicitly. No root is required to build the package.
- Distribution CI keeps the portable tarball and adds a primary `.deb`; it
  installs through apt, validates launcher/icon/installed startup, then removes
  the package. Actual Linux application validation awaits the runner.
- Local validation includes metadata/manifest unit tests, TypeScript checks and
  actionlint. A native Debian arm64 Docker container built a package around a
  `/bin/true` fixture, installed it with apt (resolving dependencies), validated
  its desktop entry/icon and binary contents, and uninstalled it successfully.
  This exercises real Debian tooling, not GPUI rendering or the Linux app build.

## 2026-10-21: Native desktop distribution CI

- Expanded `macos-distribution.yml` (display name: Desktop distribution) with
  native Ubuntu 24.04 x64 and Windows x64 packaging jobs. Existing Apple Silicon
  packaging, optional Developer ID signing/notarization and ZIP output remain.
- Linux/Windows package the Solid workspace as an embedded executable, run
  manifest collection tests and isolated startup validation, then upload unsigned
  tar.gz/ZIP artifacts. No cross-compilation or external app source is required.
- `scripts/check-packaged.ts` copies only the executable into a temporary working
  directory, clears inherited Lua/Node module paths and runs `--check` with a
  timeout. All three platform jobs use it. It does not test real GUI painting.
- Local validation: actionlint, TypeScript checking, manifest tests and isolated
  macOS packaged startup pass. Linux/Windows success is pending the first remote
  workflow run; this machine cannot verify those native builds.

## 2026-10-21: Manifest-driven standalone packaging

- `gpuix.json` defines application identity, entry, explicit payload files,
  an independent app icon and macOS bundle metadata. `scripts/app.ts` provides
  `dev` and `package`; workspace/tray start recipes use the manifests.
- Packaging builds an optimized runner with a compile-time embedded payload.
  `gpx` extracts to an owned temporary directory for existing module and asset
  loaders, keeping it alive through the application loop. Normal exit cleans up.
  macOS bundles contain the executable, plist and app icon, not source sidecars.
- `just package MANIFEST` is generic; `just package-solid` retains its existing
  signing/notarization/ZIP flow using the generic packager underneath.
- Runtime permissions remain deferred in TODO.md. Unknown manifest fields,
  including permission declarations, fail rather than implying a sandbox.
- Validation: 10 optimized launcher/extraction tests and 3 manifest/collection
  tests pass; TypeScript checks pass. Lua standalone and JavaScript app-bundle
  outputs validate from `/` without source sidecars. The signed Solid app and
  ZIP distribution build successfully; manifest-based development bundle checks
  and all four example manifest entry checks pass without opening GUI windows.
- Follow-ups: direct archive-backed loading/compression, Windows/Linux packaging
  validation, resource limits and crash cleanup, and concurrent package-build
  isolation (commands currently share Cargo's runner output).

## 2026-10-20: Application-owned macOS trays

- Removed the experimental launcher `--tray` flag. Lua applications own trays
  through `create_tray(app, options)`, `update_tray`, `remove_tray`, and `quit(app)`.
  Embedded React/Solid apps use framework-neutral `@gpuix/native/app` exports.
- Both paths share native tray management, checked/disabled menu items, explicit
  disposal, stale-callback rejection and last-window lifetime policy. JavaScript
  retains its runtime/framework tree while windows close and reopen.
- `just start-tray` and `just start-tray-js` run application-controlled demos.
  The API remains macOS/gpx-only, with flat menus and an initial window required.
  JS apps with active trays use full reloads rather than Fast Refresh to avoid
  duplicating app-scope resources. Lua failed entry reloads roll back tray state.
- Vendored gpui-tray 0.1.4 with a one-line macOS menu validation fix: AppKit
  otherwise auto-enables disabled entries when opening a menu. See vendor README.
- Native menu callbacks were exercised through AppKit under LLDB: Lua reopen,
  JS reopen with React count preserved, checked-state updates, disabled-state
  validation and Quit. The notch can hide icons on crowded menu bars.
- Follow-ups: Bun/Node host support, tray-only startup, nested menus, JS
  resource-aware Fast Refresh, and preserving GPUI image scale in tray icons.

## 2026-10-20: Mahina/Lune embedding probe

- Added the opt-in `mahina-experiment` feature and `bench_mahina` Rust example.
  Uses the release v0.1.2 embedding API and real retained-tree operations;
  the normal `gpx` runner is unchanged. This is headless, not a general
  Mahina window/LuaX/widget host yet.
- The existing Lua signal core runs under Mahina. Two table-call shorthand
  expressions now use explicit parentheses, also valid in Lua 5.4/LuaJIT.
- Three lifecycle tests passed with each Lua backend build, covering Mahina
  tracing on/off and Lua 5.4 or LuaJIT. Timed runs at 1k/10k nested keyed
  rows also passed identity, two-label update and disposal assertions.
- `WARMUP` defaults to 200; selection and full-list reversal are distinct
  measurements. Native Mahina JIT is Linux x86-64 only, not this Mac.
  Local measurements were noisy/contended and do not establish a speed win.
- Full integration direction, reproducible commands and measurement details:
  `docs/design/mahina-integration.md`. Next slice is a Mahina signal-host
  adapter with validated handles, styles/props, guest events and native
  window integration, not a rewrite of the Lua reactive algorithms.

This is a working handoff, not a repository overview or public API contract.
Implementation and verification notes below are dated snapshots; use
`git status` and `git log` for current local changes and push state. See
`README.md` and `AGENTS.md` for the current architecture and repository map.

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

2026-10-19: Added an opt-in `gpx --dev-server PORT` loopback HTTP API shared by
Lua and QuickJS. It enables the existing watcher and exposes authenticated
`POST /rpc` commands for health, native window discovery, retained trees,
clicks and keystrokes. `GPUIX_DEV_TOKEN` is required; see README for the wire
contract and limits. Commands run on the GPUI foreground task. Input replies
acknowledge dispatch rather than completion, so callers poll the tree.
Paint bounds and locator actions are intentionally not exposed:
the existing paint-bounds registry is not window-scoped. Preserve correct
multiwindow routing before adding those operations.
The API now also exports detailed host trees (`details: true`) and per-window
base64 PNG screenshots from a freshly drawn GPUI frame. Screenshots use GPUI image
readback with the default `test-support` feature, not desktop capture or file
writes. This is a retained host tree, not a framework component/owner tree.

Verification: 514 Rust library tests and 6 `gpx` CLI tests pass with
`--release --features quickjs`.
A rebuilt release runner passed real HTTP smoke tests against Lua, React and
Solid apps: health, windows, tree reads, counter updates from native clicks,
keystrokes, and rejection of unauthenticated requests. Input dispatch uses
`AnyWindowHandle::update`, without borrowing the root view across event
callbacks; using the typed view update there causes a GPUI entity reentry panic.
Screenshot follow-up: the same three runtimes pass live PNG capture checks
before/after counter changes, including separate captures of two Lua windows.
PNG metadata/signatures and detailed styles/events are checked; the captured
Lua PNG was also visually inspected. Capture uses public `Window::draw` plus
`render_to_image` and clears the returned arena token. It does not wait on a
platform frame callback, which can strand a capture of an inactive window.
Subtree follow-up: `subtree` selects a mounted node by `nodeId` or unique
`testId`, returning an opaque revision and only that section. `ifRevision`
returns metadata alone when unchanged, without serializing descendants.
Tokens include the tree incarnation and detail mode; there is no per-client
snapshot cache. Event-listener and testId mutations now advance subtree
revisions as well. These are conditional snapshots, not mutation patches or
visual-change subscriptions; ancestor-inherited appearance is not covered.
Verified subtree lookup and unchanged/changed conditional polling over real
HTTP in Lua, React and Solid apps; the existing PNG/input/multiwindow checks
still pass. The four subtree regression tests bring the Rust suite to 514.

The QuickJS port follows `docs/design/embedded-quickjs-runtime.md`; its
checklist is the source of truth for what's done. As of 2026-10-16,
Phases 0 to 5 are in, and Phase 6's React Fast Refresh is too (51c5012).
Other agents may have local changes; inspect `git status` before editing and
leave unrelated work alone.

Run `bun run build:embedded` in `packages/native` before a `--features
quickjs` build, and `bun run build:quickjs` before the JS suites' QuickJS
tests. React's suite runs with `bun run test` (vitest); `bun test` hangs
there. Agent-tool review subagents kept stalling; `claude -p` with the
prompt on stdin worked.

### 1. Solid hot refresh under `gpx --watch` (implemented)

2026-10-19 implementation update:

- Local Solid checkout: `../gpuix-solid-refresh`, based on pinned commit
  `4937f0a2bfddd45a65738c189075aff577530073`.
- Reviewable upstream diff: `docs/design/solid-refresh-compiler.patch`.
  Apply it from the Solid repository root with `git apply`.
- Exposes `solidjs_compiler::refresh::transform_refresh` without the `node`
  feature, returning `RefreshOutput { code, map }` and `CompileError`.
  The Node adapter retains its existing result shape. Shared path/hash
  helpers move out of the Node-only directives module; the transform itself
  is unchanged.
- Verification: 12 library tests, 3 existing host-independent interface
  tests and 5 new refresh tests pass with `--no-default-features`.
  Six refresh tests, including Node-adapter output parity, pass with
  `--no-default-features --features node,napi/dyn-symbols` (dynamic symbols
  let that Rust integration-test executable run without a Node host).
  Formatting and diff checks pass; the normal no-default-features dependency
  tree contains no NAPI dependency.
- Bart approved publishing the fork. The compiler patch is committed and
  pushed to `serialexp/solid` branch `gpuix-host-independent-refresh`, revision
  `7a236862b67c37cfda95e28ef199e9bc3251b489`; GPUIX is pinned to that revision.
  No upstream PR has been opened.
- Runtime integration now embeds Solid's development refresh runtime,
  installs per-module hot contexts, composes refresh/JSX source maps, and
  uses the existing import graph for update propagation.
- Verification: all 504 Rust library tests pass with `--release --features
  quickjs`, including all 115 QuickJS tests. Coverage includes repeated child
  edits retaining parent/sibling signals and node IDs, plain TypeScript
  propagation, compile-error recovery, disposal/data persistence, invalidation,
  entry restart, and authored-source error positions. All 36 Solid package
  tests pass. Embedded framework bundles rebuild successfully. Targeted Rust
  formatting and diff checks pass. A live GUI watch session has not been run
  for this change; the new behavior is exercised through the native retained
  tree/event tests. GPUIX changes remain uncommitted.

Bart decided on 2026-10-16: an upstream PR to solidjs/solid that makes the
compiler's refresh pass usable without napi, and `gpx` pinned to a fork's
revision until it lands. Pushing the fork and opening the PR are
outward-facing: show Bart the diff first.

Compiler baseline before extraction (historical context):
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

Original implementation plan (retained for design context):
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
