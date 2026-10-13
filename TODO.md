# TODO

- Solid structural updates walk the whole window tree. In
  `packages/native/src/lua_runtime.rs`, each `HostUpdate::Child`/`Children`
  calls `find_host_node` (search from the root for the slot), then rebuilds
  the window's whole handle map with `collect_host_handles` and its whole
  handler map with `collect_handlers`. A window with many `solid.Show` or
  `solid.For` slots, or a large tree, pays several full-tree walks per
  structural update. Index slots by element id and update only the handles
  and handlers of the subtrees that were added or removed.
- `solid.text`, `bind_style` and `bind_prop` (`packages/lua/gpuix/solid.lua`)
  call the host setter on every effect run, even when the value did not
  change, so a signal read by N rows costs N host calls per change. Found
  while benchmarking in `~/Projects/rust-js-vm`: SolidJS's universal
  renderer skips unchanged values before calling the host. Comparing with
  the previous value in the effect would avoid the calls.
- `solid.lua` has no `create_selector`. Selection-style state read by every
  row (like `selected() == index`) re-runs every row's effect on each change.
  `~/Projects/rust-js-vm` (`src/workload.rs`, `create_selector`) has a
  minimal keyed version that re-runs only the two affected rows.
- Upstream code warns on a macOS native build: `browser_platform_is_macos`
  (`custom_elements/input.rs`), `BlockTree::is_empty` (`markdown/parser.rs`)
  and `Metrics::hash_into` (`theme.rs`) are unused outside the browser or test
  builds. Fix upstream (cfg-gate them) rather than in this fork, so rebases
  stay clean.
