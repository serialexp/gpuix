# Embedded QuickJS runtime

**Status:** In progress
**Last updated:** 2026-10-16

## Implementation status

Tracking the gap between this design and what's on the main branch.

### Done

- [x] Engine comparison and design spike, outside this repo (rust-js-vm,
  see References)
- [x] Phase 0: measure `@gpuix/react` and Solid 2.0 running inside QuickJS
  on the rust-js-vm workload (rust-js-vm `src/js/react.rs`, `react/`,
  `solid2/`; results in its README, "Real React and Solid 2.0", and under
  open question 1 below. rust-js-vm is not a git repository, so there is
  no commit)
- [x] Phase 0: settle open question 1: real `@gpuix/react` (decided
  2026-10-13)
- [x] Phase 1: `script/app.rs`: window options, app commands, focus
  requests, window spec checks and a generic application registry, used by
  the Lua runtime (`refactor(native): share app registry and source watcher
  across script engines`)
- [x] Phase 1: `script/reload.rs`: source watcher and reload result types,
  used by the Lua runtime and `lua_app.rs` (same commit)
- [x] Phase 1: Lua behavior unchanged: every existing Lua test passes, on
  Lua 5.4 and LuaJIT (same commit)
- [x] Phase 2: `quickjs` cargo feature with vendored rquickjs-sys and the
  Map hash fix (`packages/native/vendor/`), excluded from wasm builds and
  from the default features until Phase 4 uses it (`feat(native): add the
  embedded QuickJS host`)
- [x] Phase 2: QuickJS host globals: timers (with Node's delay limits and
  the browser's nesting clamp), microtasks, `console` (Node-style
  formatting), `performance.now`, `queueMicrotask`, `structuredClone`
  (`src/quickjs/prelude.js`, same commit)
- [x] Phase 2: native `applyBatch` over `apply_batch_to_tree`, so
  `createMutationQueue` runs unchanged (same commit)
- [x] Phase 2: event dispatch into QuickJS: `QuickJsRuntime::dispatch_event`
  sends an `EventPayload` to the handler JS registers with
  `__gpuix.setEventHandler`, which Phase 3's `@gpuix/native` shim points at
  `createRendererState(renderer).dispatch`. `preventDefault` and
  `stopPropagation` stay in JS, as under Node, so nothing returns to Rust.
  Connecting GPUI's event callback is the app loop's job (Phase 4) (same
  commit)
- [x] Phase 2 follow-up: React updates get React DOM's priorities, so a
  click's update commits in a microtask and `dispatch_event` returns with
  it applied. `@gpuix/react`'s host config sets `supportsMicrotasks` and
  resolves the priority from the running GPUI event, as React DOM does from
  `window.event`; mouse movement and scrolling stay continuous. Decided by
  Bart on 2026-10-13, for Node and QuickJS alike (`feat(react): give event
  updates React DOM's priorities`)
- [x] Phase 3: oxc compile of app `.ts`/`.tsx`/`.mts`/`.js`/`.jsx`/`.mjs`
  files: TypeScript stripped, React automatic JSX with
  `importSource: "@gpuix/react"`, errors as `path:line:column`
  (`src/quickjs/compile.rs`, `feat(native): load TypeScript and JSX app modules in QuickJS`)
- [x] Phase 3: ESM module loader: relative and absolute app imports found the
  way Bun finds them and compiled on load, bare `react` and `@gpuix/*`
  imports resolved to the embedded bundles, anything else rejected with the
  list of what is bundled (`src/quickjs/modules.rs`,
  `QuickJsRuntime::load_module`, same commit)
- [x] Phase 3: embedded React, `@gpuix/react` and `@gpuix/native` bundles
  built by `bun run build:embedded` (`embedded/build.ts`), with a stale-bundle
  check in `build.rs` (same commit)
- [x] Phase 4: one app binary, `gpx`, that picks the engine from the entry file's
  extension (`.lua`/`.luax` or `.js`/`.jsx`/`.ts`/`.tsx`), replacing
  `gpuix-lua`; same flags (`--check`, `--watch`, window options). `just
  start`, `just start-solid` and `scripts/package-solid.sh` run `bun run
  build:embedded` first, since `build.rs` needs the bundles
  (`src/bin/gpx.rs`, `src/quickjs_app.rs`, `feat(native): add the gpx app
  binary for Lua and JavaScript apps`)
- [x] Phase 4: the embedded `@gpuix/native` shim's `init` passes the window
  options `render()` gives it to the host, which opens the window with them
  (same commit)
- [x] Phase 2 follow-up, due with Phase 4: the `NativeRenderer` window
  methods (focus, Tab, selection, scrolling, images, fonts, window
  controls, the debug overlay, `promptForPaths`) on the embedded
  `@gpuix/native` shim, run by `gpx`'s window host on the GPUI thread as on
  macOS, and `gc()` for `idle-gc.ts` (`src/quickjs/window.rs`,
  `src/quickjs_window.rs`, `embedded/native.ts`, `feat(native): give gpx
  apps the renderer's window methods`)
- [x] Phase 4: small Node/Bun shims so the example apps run under `gpx`: a
  frozen `process` snapshot (`env`, `argv`, `platform`), `import.meta.url`
  and `import.meta.main`, and `with { type: "text" }` imports of the app's
  own files (`src/quickjs/prelude.js`, `src/quickjs/modules.rs`,
  `feat(native): give gpx apps process, import.meta and text imports`)
- [x] Phase 4: React app parity tests. Each scenario runs one app from
  `packages/react/src/__tests__/fixtures/parity` under Node and under
  `gpx`'s QuickJS in the test renderer (`TestRenderer.loadJs`), drives both
  through the same window events, and compares their trees. They need a
  `.node` file built with `bun run build:quickjs`, and skip without one
  (`quickjs-parity.test.tsx`, `feat(native): compare React apps under gpx
  with Node in tests`)
- [x] Phase 5: `@gpuix/solid`, its components and `examples/solid` on
  Solid 2.0 (`solid-js`, `@solidjs/universal` and `@solidjs/compiler`
  pinned to `2.0.0-rc.15`), Node and Bun apps included. The Bun preload and
  plugin compile with `@solidjs/compiler` and load Solid's development build
  unless `NODE_ENV` is `production` (`packages/solid/`, `feat(solid)!: move
  @gpuix/solid to Solid 2.0`)
- [x] Phase 6: React Fast Refresh under `gpx --watch`: the development
  bundle set with React's development build and `react-refresh/runtime`,
  oxc's refresh transform, changed files loaded again as new module
  versions up to modules that export only components, and a fresh runtime
  when a change reaches the entry file (`src/quickjs/hot.rs`,
  `feat(native): give gpx --watch React Fast Refresh`)
- [x] Phase 5: Solid JSX compile for app files with `@solidjs/compiler`'s
  Rust crate (`solidjs-compiler`) in universal mode, pinned to the
  `solid-js@2.0.0-rc.15` git revision. The nearest tsconfig's
  `jsxImportSource` picks Solid or React, and the entry file's imports
  decide without one (`src/quickjs/compile.rs`, `src/quickjs/framework.rs`,
  `feat(native): run Solid apps under gpx`)
- [x] Phase 5: embedded `solid-js` and `@gpuix/solid` bundles, built from
  Solid's production client build by `embedded/build.ts` (same commit)
- [x] Phase 5: Solid app parity tests, under Bun and under `gpx`'s
  QuickJS, skipped without a `build:quickjs` `.node` file
  (`packages/solid/src/__tests__/quickjs-parity.test.ts`, same commit)

### Outstanding

- [ ] Phase 1: `script/events.rs` (deferred: Lua's handler lookup, handle
  routing and dirty-window tracking are Lua-only, and QuickJS keeps its
  handlers in JS; extract shared routing in Phase 2 if it overlaps)
- [ ] Phase 1 (dropped 2026-10-13): direct host and its swap-safe flush.
  Lua's Solid layer is not a direct host, and QuickJS reuses
  `apply_batch_to_tree` (see the Shared core update)
- [ ] Phase 3: compiled output cached by content hash (deferred to Phase 6,
  with hot reload: compiling is fast enough that only `--watch` restarts
  would notice)
- [ ] Phase 4 follow-up: run the parity tests in CI (deferred: CI builds
  the `.node` file without QuickJS, so they skip there; a `build:quickjs`
  step costs a second release build of the crate)
- [ ] Phase 6: Solid hot refresh under `gpx --watch`, with Solid 2.0's
  `solid-js/refresh` and its compiler's refresh pass. The pass only builds
  with the crate's napi feature, so this waits on an upstream PR exposing
  it, pinned to a fork's revision until it lands (decided by Bart on
  2026-10-16). Until then a Solid change starts the app again
- [ ] Phase 6: benchmark numbers in `docs/lua-runtime.md`'s benchmark
  table (the README's `gpx` section and the changesets are in)
- [ ] Rust-reconciler React-style API for JS, shared with Lua's hooks
  (deferred: Phase 0 showed real React is slower but acceptable; revisit
  only if apps need it)
- [ ] Bundle any npm package an app imports (deferred: a later goal, see
  open question 3)
- [ ] Decorators in app code (deferred: quickjs-ng can't run them, so they
  are a compile error; oxc's decorator transform could lower them if apps
  need them)
- [ ] System APIs for apps (deferred: none at first, see open question 4)

## Why this exists

GPUIX apps today run React or Solid in Node or Bun, and every tree change
crosses napi as a batch of mutations. The experimental Lua runtime
(`packages/native/src/lua_runtime.rs`, see `docs/lua-runtime.md`) showed that
running the UI code inside the Rust process, writing straight into the
retained tree, removes that cost and the need for a JavaScript runtime
at all. But Lua apps can't use the React and Solid packages, so the Lua
side has had to grow its own component library (`packages/lua/gpuix/`).

The rust-js-vm experiment rebuilt the Lua design independently of the
engine and drove it from both Lua and QuickJS. QuickJS kept up: within a few
percent of Lua for React-style updates, faster at full re-renders and mounts,
and real SolidJS 1.9 ran unchanged through its universal renderer. This doc
is the contract for bringing that into GPUIX: one shared Rust core for
embedded engines, and a QuickJS engine that runs JavaScript and TypeScript
apps with no Node or Bun.

## Goals

- Run a GPUIX app written in TypeScript/TSX or JavaScript/JSX from a single
  native binary, with no Node, Bun or separate build step.
- Support both programming models GPUIX already has: React-style and
  Solid-style.
- Compile JSX and TypeScript inside GPUIX, in Rust.
- Write tree changes straight into the retained tree, with no JSON or napi
  hop per update, as the Lua runtime does.
- Share the engine-independent parts with the Lua runtime instead of
  duplicating them.
- App code that runs on the embedded engine also runs under Node/Bun
  unchanged, as far as the open questions allow.

## Non-goals

- A Node-compatible runtime. No `fs`, `net`, `process` or `child_process`
  in the first version; see open question 3.
- Running in the browser. The wasm build already uses the browser's own
  engine, so all of this is left out of wasm builds, as the Lua code is.
- Replacing the Node/Bun path. It stays the main way to run GPUIX apps.
- Solid 2.0 support. `@gpuix/solid` targets Solid 1.9.

  > **2026-10-13 update:** reversed. Everything moves to Solid 2.0, including
  `@gpuix/solid` for Node and Bun apps, because 2.0 ships an official
  Rust/oxc compiler with universal mode (open question 2).

## Today

- `packages/native/src/lua_runtime.rs` (about 8,400 lines) holds
  everything for Lua: the React-style hook store, the render arena, memo
  keys and the reconciler; the host API `solid.lua` uses (`set_text`,
  `set_style`, `set_prop`, `set_children`); the application and window
  registry; module loading and hot reload. It all works on mlua values
  directly.
- `packages/native/src/lua_app.rs` and `src/bin/gpuix-lua.rs` run a Lua
  app in its own GPUI windows.
- `@gpuix/react` and `@gpuix/solid` both write through one small interface,
  `MutationRenderer` in `packages/native/js/host.ts`: `createElement`,
  `destroyElement`, `appendChild`, `insertBefore`, `setStyle`, `setText`,
  `setEventListener`, `setRoot`, `setKeyboardFocusDim`, `setCustomProp`,
  `flushMutations`, `preventDefault`, `stopPropagation`.
  `createMutationQueue` (`packages/native/js/mutations.ts`) turns those calls
  into batches for napi.
- `@gpuix/solid` compiles JSX with Babel's `babel-preset-solid` in universal
  mode (`packages/solid/src/bun-plugin.ts`). `@gpuix/react` uses
  `jsxImportSource: "@gpuix/react"`.

  > **2026-10-16 update:** `@gpuix/solid` is on Solid 2.0 now and its Bun
  > plugin compiles with `@solidjs/compiler` (napi) in universal mode; Babel
  > is gone. See Phasing item 5.

## Design

### Shared core: `packages/native/src/script/`

The engine-independent parts move out of `lua_runtime.rs` into a module
both engines use. Following rust-js-vm's `src/core/`, they are generic over
the engine's handler type instead of storing engine values directly:

- `script/direct.rs`: a host that scripts change node by node. It keeps
  parent and sibling links so a remove is constant time, writes props to the
  retained tree at once, and writes child order once per event at the flush.
  The flush first detaches every child that changes parent, then writes the
  new child lists, so a parent and child that swapped places are never inside
  each other. `solid.lua`'s host calls and the QuickJS `MutationRenderer`
  binding both use it. Modeled on rust-js-vm `src/core/direct.rs`.
- `script/app.rs`: the application and window registry, window commands and
  focus requests that `lua_runtime.rs` keeps in `LuaApplicationRegistry`,
  `LuaAppCommand` and `LuaFocusRequest` today.
- `script/events.rs`: delivering a GPUI event to the script that owns the
  element, and carrying `preventDefault` / `stopPropagation` back.
- `script/reload.rs`: the file watching and module bookkeeping the Lua
  reload uses, minus the Lua-specific state preservation.

> **2026-10-13 update:** the code doesn't fit this plan, so Phase 1 shares
> less. `solid.lua`'s host calls don't change nodes one by one: they stage
> nodes in the React-style render arena, mount them through
> `reconcile_node`, and only ever replace a slot's whole child list
> (`replace_host_children`), rejecting moves between parents. Putting Lua
> on a direct host would be a rewrite with behavior changes, and the
> parent/child swap can't happen in Lua at all. QuickJS doesn't need a
> direct host either: `MutationRenderer` has 11 methods (`preventDefault`
> and `stopPropagation` are on key events, in JS, and nothing returns to
> Rust), and gpuix already applies them with `apply_batch_to_tree`. Phase 0
> measured JSON batches at the same cost as per-call bindings, so QuickJS
> keeps `createMutationQueue` and calls `apply_batch_to_tree` through a
> native `applyBatch`. `script/` holds what both engines do need:
> `app.rs`, `events.rs` (routing and focus requests only) and `reload.rs`.

> **2026-10-13 update:** `events.rs` is deferred too. Reading the dispatch
> path showed nothing QuickJS would reuse beyond the focus-request type,
> which is in `app.rs`. Lua's module bookkeeping (`package.loaded`) stays in
> `lua_runtime.rs` for the same reason; `reload.rs` holds the source watcher
> and the reload result types.

Lua's React-style hook store, render arena and reconciler stay in
`lua_runtime.rs` unless open question 1 lands on the deferred Rust-reconciler
option, in which case they move into the core as well (rust-js-vm
`src/core/host.rs` shows the generic shape: `Host<H, S>`).

### QuickJS engine: `packages/native/src/quickjs/`

- rquickjs over quickjs-ng, behind a `quickjs` cargo feature, compiled out of
  wasm. It vendors rquickjs-sys with the Map hash fix from rust-js-vm
  (`patches/rquickjs-sys-0.14.0-map-hash-mix.diff`), which SolidJS's `For`
  needs: without it, reversing 10,000 rows takes 75 ms instead of 13 ms.
  The fix should also go upstream to quickjs-ng (needs Bart's OK).
- A native object implementing `MutationRenderer`, installed where
  `@gpuix/react` and `@gpuix/solid` look for their renderer. Each call goes
  straight into the shared direct host. `flushMutations` is the flush.
- Host globals the frameworks need: `setTimeout`/`setInterval` and their
  clear functions driven from the GPUI run loop, `queueMicrotask` and promise
  jobs run after each event, `console` to stderr, `performance.now`,
  `structuredClone`. React's scheduler falls back to `setTimeout` when
  `MessageChannel` is missing, so that isn't needed.
> **2026-10-13 update:** Phase 2 shipped as `src/quickjs/mod.rs` and
> `prelude.js`. JS reaches the tree through `__gpuix.renderer.applyBatch`
> (JSON batches, as decided under Shared core) rather than a native
> `MutationRenderer` object. The runtime borrows the caller's
> `RetainedTree` for each call that runs JS, so windows keep owning their
> trees. Timers are kept in JS and driven from Rust: `run_timers` runs the
> due ones, `next_timer` says when to wake. An exception, a failed promise
> job, an unhandled rejection or `reportError` fails the call it happened
> in. A panic in a native function fails the call even if JS catches the
> exception it turns into.

> **2026-10-16 update:** the runtime no longer borrows the caller's tree.
> It has a `Host` that owns the tree and runs the renderer's window
> methods: a `TreeHost` until the app's window opens, then `gpx`'s
> `WindowHost`, which locks the window's tree only while a batch applies.
> Borrowing kept the tree locked for the whole call, so a window method
> that reads the tree (focusing inside a virtual list, `setImage`) would
> have deadlocked. Calls made before the window opens (an effect's
> `focusElement`) are queued and run once it has; queries answer as an
> unpainted window would, and the window's size throws, as napi does while
> a window opens.

- rust-js-vm's `src/js/raw.rs` fast path (reading props as atoms and integer
  tokens from value tags) took 10–15% off full renders. It only applies to
  the deferred Rust-reconciler option, since the `MutationRenderer` binding
  takes scalar arguments.

### Compiling app code with oxc

- `.ts`, `.tsx`, `.js`, `.jsx`, `.mjs` app files compile on load with
  `oxc_transformer`: TypeScript stripped, JSX to the automatic runtime.
- React: `importSource: "@gpuix/react"`, the same setting app tsconfigs use
  today.
- Solid: needs Solid's own JSX transform, not the automatic runtime; see open
  question 2.

  > **2026-10-13 update:** Solid 2.0's `@solidjs/compiler` (`packages/compiler` in
  solidjs/solid, an `rlib` crate) with `generate: "universal"`. It builds on
  oxc 0.144, so GPUIX's own oxc use pins that version too. The crate isn't on
  crates.io and its Rust API is marked unstable before 2.0 is final, so it is
  pinned to an exact git revision and updated on purpose.
- Compiled output is cached by content hash under the app's cache directory,
  so a `--watch` restart only recompiles changed files.

> **2026-10-14 update:** shipped in `packages/native/src/quickjs/compile.rs`.
`.mts` compiles too; `.d.ts` files and other extensions are rejected, and
`.js` files may hold JSX, as in Bun. Nothing is lowered, since QuickJS runs
current JavaScript. `enum`, `namespace` and constructor parameter properties
compile. What QuickJS can't run (decorators, `accessor` fields) and
TypeScript's CommonJS forms (`import x = require()`, `export =`) fail the
compile at their position. Imports are kept unless they are marked `type`, as
`verbatimModuleSyntax` (which GPUIX's own packages use) keeps them, so an
import that only looks unused still runs; oxc's default would drop it. Bart
chose this over dropping unused imports or reading the app's tsconfig
(2026-10-14). A type imported without `type` fails at load. Errors read
`path:line:column: message`, then one line for each other place the error
points to; columns count UTF-16 code units, as editors do. Each compile also
keeps a source map, and the prelude's `Error.prepareStackTrace` maps stack
trace positions in app files back to the source. The cache moved to Phase 6.

> **2026-10-14 update:** the mapping moved from the prelude's
> `Error.prepareStackTrace` into a C hook, `JS_SetSourcePositionFunc`, added
> by a second vendored rquickjs-sys patch (`source-position`, see
> `packages/native/vendor/README.md`). A JavaScript hook can't run when the
> stack is full, so QuickJS left a stack overflow with no stack at all. The
> C hook also maps the positions an app's own `Error.prepareStackTrace`
> gets; an app that sets one still loses a stack overflow's frames, as in
> stock QuickJS. oxc now checks regular expressions too, so a bad one fails
> the compile at its position rather than in QuickJS with none. Bart chose this over a fallback-only patch or mapping only in
> GPUIX's own error output.

### Modules

QuickJS runs ES modules natively. A loader resolves:

- relative imports to app files, compiled on load;
- `@gpuix/react`, `@gpuix/solid`, `@gpuix/native/host`, `react`,
  `react-reconciler`, `solid-js` and their subpaths to embedded bundles.
  These are built when the native package is built (prebundled from
  `node_modules`, in ESM, with CommonJS already converted), and embedded with
  `include_str!`. `build.rs` fails the build if a bundle is older than its
  sources, as rust-js-vm's `build.rs` does.

That fixes the framework versions to the ones GPUIX was built with, like the
Lua runtime's bundled `gpuix.*` modules.

> **2026-10-13 update:** only the embedded framework bundles for the first version.
Bundling any package an app imports is a later goal (open question 3).

> **2026-10-14 update:** shipped as `packages/native/src/quickjs/modules.rs`
and `packages/native/embedded/build.ts`:
>
> - One `Bun.build` with code splitting covers every importable module, so
>   React exists once: `react`, `react/jsx-runtime`, `@gpuix/react` and its
>   subpaths, and `@gpuix/native` and its subpaths except `./testing` and
>   `./automation`, which need Node. 16 modules, about 500 KiB, built from
>   React's production build. `react-reconciler` and `scheduler` are inside
>   the bundles but aren't importable themselves.
> - React is CommonJS, so its entries re-export the names its production
>   build has.
> - `@gpuix/native` is a shim (`embedded/native.ts`) over the host's
>   `__gpuix` object. The automation client is replaced by a stub that
>   throws, which keeps zod out of the bundle.
> - The output (`embedded/dist/`) isn't committed, like `build:js`'s. Its
>   manifest records a hash of every file the build read, and `build.rs`
>   fails with the command to run when the bundle is missing or stale.
> - App files are named by their real path, so one file reached two ways is
>   one module. Relative imports resolve as Bun resolves them: the exact
>   file, `.js` naming `.ts` or `.tsx`, an added extension, then `index.*`,
>   trying extensions in Bun's order for `import` (tsx, jsx, mts, ts, mjs,
>   js). `with { type: ... }` imports are rejected.
> - The Solid bundles come with Phase 5.
>
> **2026-10-16 update:** they did. `solid-js` and `@gpuix/solid`'s
> subpaths are entries, all but `testing`, `automation`, `preload` and
> `bun-plugin`, as for React; `@solidjs/signals` and `@solidjs/universal` are
> bundled in but not importable. They embed Solid's production build, so
> `gpx` apps run without Solid's development checks, as with `NODE_ENV=production`
> under Bun. `@solidjs/signals` is built first without code splitting:
> Bun 1.4.0's splitting ran the modules of its import cycle in the wrong
> order (see TODO.md), and the build now fails on any import cycle, which
> found and removed one in `@gpuix/react`.

### The `gpuix-js` binary

Mirrors `gpuix-lua`: `gpuix-js [--check] [--watch] [--title] [--width]
[--height] app.tsx`, plus app-bundle launch from `Resources/app/main.tsx`.
`--check` compiles and loads without opening a window. A napi
`loadJs`/`loadTsx` on `TestGpuixRenderer` gives tests the same entry point
the Lua tests use.

> **2026-10-13 update:** one binary for both engines instead, picking the engine from
the entry file's extension (open question 5). It replaces `gpuix-lua`. The
name is still to be picked; `cli/` already ships the npm `gpuix` command.

> **2026-10-14 update:** the binary is `gpx` (decided by Bart): short, and
separate from the npm `gpuix` command that creates apps.

> **2026-10-14 update:** shipped as `gpx` in `src/bin/gpx.rs`, with the
JavaScript app loop in `src/quickjs_app.rs`. Details the design left open:
> - `gpx` loads the app and runs its zero-delay timers (at most 1,000
>   rounds, so an app that keeps queuing `setImmediate` still opens) before
>   opening the window, so `render()` has passed its window options and
>   committed its first tree by then. An app that never calls `render()`
>   fails with an error instead of an empty window, on reload too.
> - `--title`, `--width` and `--height` beat the options the app passes to
>   `render()`. Without a title, the window is named after the entry file,
>   as Lua windows are.
> - `--watch` reloads JavaScript apps by starting a fresh runtime in the same
>   window, so app state resets (Fast Refresh is Phase 6). It watches the
>   JavaScript and TypeScript files in the entry file's directory tree,
>   skipping `node_modules` and dot directories (for Lua apps too), so an
>   import from outside that directory doesn't trigger a reload. Each view
>   gets its own event channel, so a reloaded app never sees events meant
>   for the old tree, and closing the window ends the app's task.
> - In the window's loop, a zero-delay timer yields to GPUI once before it
>   runs, so an app that keeps queuing them (React's scheduler during a long
>   render) still paints and takes input.
> - Window setup is shared with the napi renderer
>   (`renderer::init_app`, `renderer::open_main_window`), which also gave Lua
>   apps the `img` element's HTTP client they were missing.
> - The `quickjs` feature stays out of the default features: the `gpx`
>   binary requires it, and the napi addon builds don't need QuickJS or the
>   embedded bundles.
> - No napi `loadJs`/`loadTsx` yet; the React parity tests will decide
>   whether they need one.
>
> **2026-10-16 update:** they did. `TestGpuixRenderer.loadJs` starts an app
> as `gpx` does, in the test window, and exists only when the `.node` file
> is built with the `quickjs` feature (`build:quickjs`), so published
> packages don't carry QuickJS. The tests found one difference: under Node,
> `init` opens the window before the first render, but `gpx` opens it after,
> so `useWindowSize` started at its 800x600 fallback. Before the window
> opens, the host now answers with the size it will open at: the app's
> options with the command line's on top, or on reload the window's current
> size.

## Phasing

0. Measure first. Bundle `@gpuix/react` and `@gpuix/solid` into a scratch
   copy of rust-js-vm, run them over a `MutationRenderer` shim, and time the
   existing workload (select and reverse, 10,000 rows) against the rust-js-vm
   numbers. This answers open question 1 before anything moves in GPUIX.

   > **2026-10-13 update:** `@gpuix/solid` is still on Solid 1.9, so the Solid
   > half measures Solid 2.0 directly: rust-js-vm's `solid/` renderer and
   > workload moved to `@solidjs/universal` and compiled with
   > `@solidjs/compiler`.
1. Shared core. Extract `script/` from `lua_runtime.rs` and move Lua onto it,
   with no behavior change: every existing Lua test passes unchanged.
2. QuickJS host. Feature flag, globals, the `MutationRenderer` binding and
   event dispatch, tested from Rust with hand-written JS.
3. Compile and load. oxc transform, module loader, embedded bundles.
4. React apps end to end: `gpuix-js`, and the React host-level tests run
   against the QuickJS host.

   > **2026-10-16 update:** the parity tests run their own scenarios in both
   > engines rather than the React suite itself, which drives React from the
   > test process through `createTestRoot`. Each scenario covers what a group
   > of those tests checks: events and state, keyed moves, effect order,
   > text input, focus and keys, and window methods.
5. Solid apps end to end, once open question 2 is settled.

   > **2026-10-13 update:** this phase starts by moving `@gpuix/solid` to Solid 2.0.

   > **2026-10-16 update:** that move is done, at `2.0.0-rc.15`. Two 2.0
   > behaviours shaped it. A signal write shows up after Solid's next
   > microtask flush, so the root flushes Solid after each native event's
   > handlers (through `createRendererState`'s `schedule` hook, as React's
   > discrete events do), and components that wrote then read back keep the
   > truth in plain variables (`AnimatePresence`) or compute from the new
   > value (`Combobox.setInput`). And `useContext` throws without a
   > provider, so optional contexts default to `null`. The port also fixed
   > props that controls copied with `{...props}`, which lost reactivity
   > under 1.x too.

   > **2026-10-16 update:** Solid apps run under `gpx`. Which framework a
   > file's JSX compiles for comes from the nearest `tsconfig.json` or
   > `jsconfig.json` (`compilerOptions.jsxImportSource`, `extends`
   > followed), as TypeScript reads it; with none, the app is Solid if its entry
   > file imports `@gpuix/solid` or `solid-js`, and React otherwise
   > (decided by Bart on 2026-10-16). Solid's compiler runs on the source
   > first, then oxc strips TypeScript, and the two source maps are
   > combined so errors and stack traces point at the source. Two bugs
   > turned up on the way: quickjs-ng read an accessor descriptor from a
   > Proxy's `getOwnPropertyDescriptor` trap as a data descriptor, which
   > broke Solid's `merge`/`omit` proxies (a third vendored patch), and an
   > empty text node, which Solid inserts for each `<Show>` or `{null}`
   > hole, took a flex `gap` slot (now it takes no part in layout, as in a
   > browser). An error in a Solid flush that no `onUncaughtError` takes is now
   > logged, as `@gpuix/react` logs its uncaught errors, rather than
   > dropped.
6. Hot reload, docs, changeset, benchmarks.

   > **2026-10-16 update:** React Fast Refresh shipped as follows. Bart
   > chose to embed React's development build for `--watch` only (Fast
   > Refresh needs it), so `embedded/build.ts` writes a production and a
   > development set, both embedded (the development one adds about
   > 1.2 MB, as the bundles aren't minified). `--watch` runs the
   > development set, Solid's development build included, and compiles
   > React files with oxc's refresh transform, its calls written as
   > `import.meta.$RefreshReg$`/`$RefreshSig$`, which the loader sets per
   > module, so no lines are added and the position maps hold. QuickJS can't
   > run a module again in place, so a changed file loads again as a new
   > module (`path?hot=N`), and imports resolve to the newest version.
   > Like Vite's, an update stops at modules that export only components,
   > the same names as before, and goes up the import graph past any other;
   > reaching the entry file starts the app again, as before. It works out
   > every module it loads again before running any, so each runs once,
   > after the new versions of what it imports, and a failed update's
   > changes carry over to the next. Old modules' side effects (timers
   > they started) stay, and old modules stay in memory, as with other
   > hot reloaders.
   >
   > For Solid, research found Solid 2.0 has its own hot refresh
   > (`solid-js/refresh`, in its development build) and a Rust transform
   > for it in the compiler crate, but only behind the crate's napi
   > feature. Unlike React's, the edited component itself remounts; its
   > parents and siblings keep their state. Bart chose an upstream PR
   > exposing the pass, with a fork pinned meanwhile (2026-10-16).

## Open questions for review

1. **React-style in QuickJS: real React, or a Rust reconciler?**
   - Real `@gpuix/react` (recommended, pending Phase 0): every component
     (`Button`, `Combobox`, `Dialog`, …), hook and behavior is the one Node
     apps get, and app code is portable. Cost: React's reconciler runs in an
     interpreter. Not measured yet.
   - A Rust-side reconciler with a hooks API on a `ui` global, shared with
     Lua's (rust-js-vm's JS React-style mode): measured at 2.8 ms per
     selection change at 10,000 rows with `memoBatch`. But it's a third
     component API, and it needs its own component library, as Lua does.
   - Both, real React first: the deferred bullet above.

   > **2026-10-13 update:** measure first (Phase 0), then decide.

   > **2026-10-13 update:** measured. At 10,000 rows, real React in QuickJS
   > takes 224 ms per selection change with every row re-rendering and
   > 34 ms with `React.memo` rows, against 14.8 ms and 3.7 ms for the Rust
   > reconciler on the same engine (9–15 times slower), and mounts in
   > 340–520 ms against 20–40 ms. Reversing all rows takes 2.4 s: React's
   > own algorithm grows roughly with the square of the row count, which the
   > same bundle shows under Bun too (141 ms there), and QuickJS runs React
   > about 16 times slower than Bun's JIT. The two transports cost the same,
   > so the embedded host can keep `applyBatch`'s JSON batches. Solid 2.0
   > costs 37 µs per selection change with a keyed selector, 22 ms with
   > every row re-running (1.x: 13 ms), and matches 1.x on reverse
   > (14–15 ms). Two Solid 2.0 behaviours the QuickJS host has to allow for:
   > it renders `""` as no text node, and removed nodes stay in reference
   > cycles until QuickJS's cycle collector runs, so the host must run it
   > (gpuix's idle GC would do). Full tables: rust-js-vm README, "Real React
   > and Solid 2.0".

   > **2026-10-13 update:** decided: real `@gpuix/react`. Its cost shows up
   > during development, so app authors can see it and design for it; React
   > apps need more care with large lists than Solid apps do. The Rust
   > reconciler stays deferred.
2. **How to compile Solid JSX in Rust?** `@gpuix/solid` needs
   `babel-preset-solid` with `generate: "universal"`.
   - Add universal mode to solid-jsx-oxc (Aeolun/solid-jsx-oxc, an oxc
     port of dom-expressions; its TODO says universal currently produces DOM
     output). Real work, but it's the long-term answer for 1.x, and the fork
     is Bart's.
   - Run Babel and `babel-preset-solid` inside QuickJS at load time, with the
     output cached. Works now, but adds a large bundle and slow first loads.
   - Use `@gpuix/solid`'s `jsx-runtime.ts` through oxc's automatic runtime.
     Works today with no compiler work, but props are read once at creation,
     so reactive values must be written as functions (`{() => count()}`).
     That's a different dialect from Solid under Node.
   - Wait for Solid 2.0's official oxc compiler (`@solidjs/compiler` on
     solidjs/solid `next`). Universal support there is unclear, and it means
     moving `@gpuix/solid` to 2.0.

   > **2026-10-13 update:** move everything to Solid 2.0, `@gpuix/solid` included. Its
   official compiler supports universal mode (README: `generate: "dom"`,
   `"ssr"`, `"universal"` or `"dynamic"`), so solid-jsx-oxc needs no work, and
   nothing is built that 2.0 would replace. Solid 2.0 was at `2.0.0-rc.14`.

   > **2026-10-16 update:** `@gpuix/solid` moved at `2.0.0-rc.15`, the
   > release current then; the Rust crate pin should match it.
3. **Which npm packages can apps import?** The embedded bundles cover the
   frameworks. Other packages could be:
   - not supported in the first version (recommended, as for Lua);
   - resolved from the app's `node_modules` at load time, ESM only;
   - bundled by an embedded bundler (rolldown is built on oxc), which handles
     CommonJS but is a large dependency.

   > **2026-10-13 update:** frameworks only for now; eventually apps must be able to
   bundle anything.
4. **What system APIs do apps get?** Nothing beyond the globals above, or a
   small `gpuix:fs` / `gpuix:process` set, or Node-named shims. Lua apps
   currently have the Lua standard library, including `io`.

   > **2026-10-13 update:** nothing at first.

   > **2026-10-14 update:** the example apps use `process.env`,
   > `import.meta.main` and Bun text imports, so `gpx` gets small shims for
   > those three (decided by Bart). npm packages stay out.

   > **2026-10-16 update:** `process` also carries `argv` and `platform`.
   > Code that finds a `process` often reads them too
   > (`process.argv.slice(2)`, `process.platform === "win32"`), and an
   > `argv` that is missing throws. `gpx` passes the app no arguments of its
   > own, so `argv` is just the binary and the entry file. It holds data
   > only: no `on`, `exit` or streams. `@gpuix/react` now checks for
   > `process.on` rather than for `process` before it installs its
   > uncaught-error handlers, and React and Solid check for `process.exit`
   > before they exit on window close.
5. **One binary or two?** `gpuix-js` next to `gpuix-lua`, or one `gpuix-run`
   that picks the engine from the file extension.

   > **2026-10-13 update:** one binary that picks the engine from the file type.

## References

- rust-js-vm (`~/Projects/rust-js-vm`): README results and design notes;
  `src/core/` (shared core), `src/js/` (QuickJS binding and SolidJS host),
  `solid/src/renderer.js` (SolidJS universal renderer over a direct host),
  `patches/rquickjs-sys-0.14.0-map-hash-mix.diff`.
- `docs/lua-runtime.md`: the Lua runtime this generalizes.
- [Aeolun/solid-jsx-oxc](https://github.com/Aeolun/solid-jsx-oxc):
  `packages/solid-jsx-oxc/TODO.md` on universal mode.
- [solidjs/solid#3599](https://github.com/solidjs/solid/pull/3599): Solid
  2.0's Babel and Rust/oxc compilers kept in parity.
- solidjs/solid `next` branch, `packages/compiler/README.md`: the
  `@solidjs/compiler` options, including universal mode.
- [oxc transformer](https://oxc.rs/docs/guide/usage/transformer): TypeScript,
  JSX automatic runtime, React Refresh.
