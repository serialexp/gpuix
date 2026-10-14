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
  Both predate the QuickJS work and come from upstream code. Neither runs in
  CI, so they accumulate. Decide whether to fix them here or upstream, then
  add both checks to CI.
- Offer the quickjs-ng `Map`/`Set` hash fix upstream (rust-js-vm
  `patches/rquickjs-sys-0.14.0-map-hash-mix.diff`, repro in
  `examples/map_hash.rs`): rebase onto quickjs-ng main, prepare the PR text,
  and Bart opens it (the local `gh` token can't reach quickjs-ng). Once
  rquickjs ships it, gpuix can drop its vendored rquickjs-sys.
- Lua Solid slots after a failed window render: the rows a `solid.For`
  queued stay staged, and the next change to that list passes them to
  `set_children`, which rejects them because they are neither mounted nor
  from the current render. Accept staged nodes that a queued structural
  update already claimed, or flush the window's queued updates before its
  next event (`packages/native/src/lua_runtime.rs`, `set_children`).
