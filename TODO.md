# TODO

- Upstream code warns on a macOS native build: `browser_platform_is_macos`
  (`custom_elements/input.rs`), `BlockTree::is_empty` (`markdown/parser.rs`)
  and `Metrics::hash_into` (`theme.rs`) are unused outside the browser or test
  builds. Fix upstream (cfg-gate them) rather than in this fork, so rebases
  stay clean. The wasm build (`cargo +nightly check --target
  wasm32-unknown-unknown --no-default-features --lib`) has 21 more, the
  same kind: `automation.rs`'s key dispatch, `GpuixView::register_font`,
  `request_focus` and its virtual-list helpers, and others with no browser
  caller.
- `cargo clippy --lib --tests` in `packages/native` fails with about 60
  errors under `#![deny(clippy::all)]` (too many arguments, needless borrows,
  derivable impls, elidable lifetimes and more), and `cargo fmt --check`
  reports drift in `custom_elements/img.rs`, `custom_elements/input.rs`,
  `renderer.rs`, `retained_tree.rs`, `test_renderer.rs` and `text/search.rs`.
  Both predate the QuickJS work and come from upstream code. CI runs the
  Rust tests but neither check, so they accumulate. Decide whether to fix them here or upstream, then
  add both checks to CI.
- Offer the quickjs-ng `Map`/`Set` hash fix upstream
  (`packages/native/vendor/rquickjs-sys-0.14.0-map-hash-mix.diff`, repro in
  `packages/native/vendor/map-hash-repro.js`, numbers in
  `docs/design/quickjs-phase0-results.md`): rebase onto quickjs-ng main, prepare the PR text,
  and Bart opens it (the local `gh` token can't reach quickjs-ng). Offer the
  `source-position` hook (`packages/native/vendor/README.md`) the same way.
  Once rquickjs ships both, gpuix can drop its vendored rquickjs-sys.
- Lua Solid slots after a failed window render: the rows a `solid.For`
  queued stay staged, and the next change to that list passes them to
  `set_children`, which rejects them because they are neither mounted nor
  from the current render. Accept staged nodes that a queued structural
  update already claimed, or flush the window's queued updates before its
  next event (`packages/native/src/lua_runtime.rs`, `set_children`).
- The `rust-test` CI job runs the native crate's tests on macOS and Linux
  only. Add Windows once someone checks that the vendored LuaJIT and
  quickjs-ng build there with MSVC.
- QuickJS cuts module names to 64 bytes (`ATOM_GET_STR_BUF_SIZE`) in some
  of its own messages, such as "Could not find export 'x' in module ...",
  and app module names are absolute paths. Show them in full, by a patch or
  by rewriting the message where GPUIX reports it.
- `gpx --watch` watches the JavaScript and TypeScript files in the entry
  file's directory tree, so a JavaScript app doesn't reload when a module
  from outside it (`../shared/x.ts`) or a text import (`./notes.md`)
  changes. Watch the files the QuickJS loader actually loaded instead (it
  knows their paths).
- Errors thrown from Solid TSX under Bun report line numbers in the
  compiled output, not the source: `@gpuix/solid`'s Bun plugin returns the
  compiler's code without a source map, and Bun ignored an inline
  `sourceMappingURL` in plugin output when tried. Find a way to hand Bun
  the compiler's map (`sourceMap: true` returns one).
- Bun 1.4.0's code splitting runs the modules of an import cycle in the
  wrong order when two entries share them (`@solidjs/signals`' core.js
  ran before scheduler.js). `packages/native/embedded/build.ts` works
  around it and fails on any other cycle. Report it to Bun with the small
  reproduction (two entries, `export * from` solid-js and from
  `@solidjs/universal`) and drop the workaround once it's fixed.
- After a `gpx --watch` hot update, stack traces name the new module
  versions with their `?hot=N` (`counter.tsx?hot=3:4:7`), which editors
  don't open as a file link, and the runtime keeps a position map for each
  version. QuickJS names a module's functions by its module name, so drop
  the suffix where traces are built (the source-position patch could
  rewrite the file name too) and free old versions' maps.
- `gpx` reads a JavaScript app's `tsconfig.json` to pick Solid or React,
  but `--watch` doesn't watch it, so changing `jsxImportSource` takes a
  restart. Add the configs it read to what it watches.
- `cargo clippy --features test-support,quickjs --all-targets` in
  packages/native reports about 58 lints (needless `return`s, too many
  arguments, `sort_by_key`, complex types), and CI doesn't run clippy.
  Clear them, then add a clippy step to CI. `cargo fmt --check` fails
  there too, on older code (renderer.rs, retained_tree.rs, img.rs,
  input.rs, test_renderer.rs, text/search.rs); format it in a commit of
  its own and check it in CI.
