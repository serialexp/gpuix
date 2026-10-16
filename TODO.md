# TODO

- Upstream code warns on a macOS native build: `browser_platform_is_macos`
  (`custom_elements/input.rs`), `BlockTree::is_empty` (`markdown/parser.rs`)
  and `Metrics::hash_into` (`theme.rs`) are unused outside the browser or test
  builds. Fix upstream (cfg-gate them) rather than in this fork, so rebases
  stay clean.
- `cargo clippy --lib --tests` in `packages/native` fails with about 60
  errors under `#![deny(clippy::all)]` (too many arguments, needless borrows,
  derivable impls, elidable lifetimes and more), and `cargo fmt --check`
  reports drift in `custom_elements/img.rs`, `custom_elements/input.rs`,
  `renderer.rs`, `retained_tree.rs`, `test_renderer.rs` and `text/search.rs`.
  Both predate the QuickJS work and come from upstream code. CI runs the
  Rust tests but neither check, so they accumulate. Decide whether to fix them here or upstream, then
  add both checks to CI.
- Offer the quickjs-ng `Map`/`Set` hash fix upstream (rust-js-vm
  `patches/rquickjs-sys-0.14.0-map-hash-mix.diff`, repro in
  `examples/map_hash.rs`): rebase onto quickjs-ng main, prepare the PR text,
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
- `examples/` doesn't typecheck (`bunx tsc -p examples --noEmit`: 51
  errors, mostly "cannot find module" for asset imports in `chat.tsx` and
  `mail.tsx`, plus missing types for `diff`), and CI never runs it. Fix the
  errors, then add the typecheck to CI next to the example-app one.
- QuickJS cuts module names to 64 bytes (`ATOM_GET_STR_BUF_SIZE`) in some
  of its own messages, such as "Could not find export 'x' in module ...",
  and app module names are absolute paths. Show them in full, by a patch or
  by rewriting the message where GPUIX reports it.
- QuickJS app modules have `import.meta` but no `import.meta.url`, so
  `new URL("./icon.png", import.meta.url)` fails. Set it to the module's
  `file://` URL when the loader declares a module (Phase 4 or 5).
- `gpx --watch` watches the files in the entry file's directory tree, so a
  JavaScript app that imports from outside it (`../shared/x.ts`) doesn't
  reload when that file changes. Watch the modules the QuickJS loader
  actually loaded instead (it knows their paths).
