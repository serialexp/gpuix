# Embedded QuickJS runtime

**Status:** In progress
**Last updated:** 2026-10-13

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

### Outstanding

- [ ] Phase 1: `script/events.rs` (deferred: Lua's handler lookup, handle
  routing and dirty-window tracking are Lua-only, and QuickJS keeps its
  handlers in JS; extract shared routing in Phase 2 if it overlaps)
- [ ] Phase 1 (dropped 2026-10-13): direct host and its swap-safe flush.
  Lua's Solid layer is not a direct host, and QuickJS reuses
  `apply_batch_to_tree` (see the Shared core update)
- [ ] Phase 2 follow-up: the optional `NativeRenderer` methods on
  `__gpuix.renderer` (`focusNext`, `focusPrevious`, `getFocusedElementId`
  and the rest), which `renderer-state.ts` uses for Tab and dismiss layers,
  and a GC hook for `idle-gc.ts` and Solid 2.0's cycles. Due with Phase 4,
  when a window exists to answer them
- [ ] Phase 3: oxc compile of app `.ts`/`.tsx`/`.jsx` files (TypeScript strip,
  React automatic JSX with `importSource: "@gpuix/react"`)
- [ ] Phase 3: ESM module loader: relative app modules compiled on load, bare
  `@gpuix/*`, `react` and `solid-js`/`@solidjs/*` imports resolved to
  embedded bundles
- [ ] Phase 3: embedded framework bundles built at package build time, with a
  stale-bundle check in `build.rs`
- [ ] Phase 4: one app binary that picks the engine from the entry file's
  extension (`.lua`/`.luax` or `.js`/`.jsx`/`.ts`/`.tsx`), replacing
  `gpuix-lua`; same flags (`--check`, `--watch`, window options)
- [ ] Phase 4: React app parity tests (the React test suite's host-level cases
  run against the QuickJS host)
- [ ] Phase 5: move `@gpuix/solid` and its components to Solid 2.0 (Node
  and Bun apps included), compiled with `@solidjs/compiler`
- [ ] Phase 5: Solid JSX compile for app files with `@solidjs/compiler`'s
  Rust crate in universal mode, pinned to a git revision
- [ ] Phase 5: Solid app parity tests
- [ ] Phase 6: hot reload: React Fast Refresh via oxc's refresh transform;
  Solid remounts
- [ ] Phase 6: README section, changeset, and benchmark numbers in
  `docs/lua-runtime.md`'s benchmark table
- [ ] Rust-reconciler React-style API for JS, shared with Lua's hooks
  (deferred: Phase 0 showed real React is slower but acceptable; revisit
  only if apps need it)
- [ ] Bundle any npm package an app imports (deferred: a later goal, see
  open question 3)
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

### The `gpuix-js` binary

Mirrors `gpuix-lua`: `gpuix-js [--check] [--watch] [--title] [--width]
[--height] app.tsx`, plus app-bundle launch from `Resources/app/main.tsx`.
`--check` compiles and loads without opening a window. A napi
`loadJs`/`loadTsx` on `TestGpuixRenderer` gives tests the same entry point
the Lua tests use.

> **2026-10-13 update:** one binary for both engines instead, picking the engine from
the entry file's extension (open question 5). It replaces `gpuix-lua`. The
name is still to be picked; `cli/` already ships the npm `gpuix` command.

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
5. Solid apps end to end, once open question 2 is settled.

   > **2026-10-13 update:** this phase starts by moving `@gpuix/solid` to Solid 2.0.
6. Hot reload, docs, changeset, benchmarks.

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
