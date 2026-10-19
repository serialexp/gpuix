# QuickJS Phase 0 measurements

**Status:** Done (a record; nothing here is built or run any more)
**Last updated:** 2026-10-19

The measurements behind `embedded-quickjs-runtime.md`'s engine choice,
Phase 0 and open question 1. They come from rust-js-vm, a standalone
experiment crate that rebuilt gpuix's embedded Lua design independently of
the engine, drove it from Lua (mlua) and QuickJS (rquickjs over quickjs-ng),
and timed React-style and Solid-style UI code writing straight into a
Rust-owned tree. That crate was never a git repository and has been removed;
this is its README's record of what it measured and found, moved here on
2026-10-19. Commands and file names below (`src/core/direct.rs`, `src/js/raw.rs`,
`renderer.js`, `solid.lua`'s copy) refer to that crate.

What gpuix kept from it: the quickjs-ng Map hash fix (vendored, with
`packages/native/vendor/map-hash-repro.js`), the decision to run real
`@gpuix/react`, and Solid 2.0. Its review fixes to the Lua reconciler
already hold in gpuix's `lua_runtime.rs` (see
`memo_hits_survive_their_parent_changing_identity` and
`a_failed_reconcile_recovers_on_the_next_render`).

## Measurements

One sample is one click on the root: the handler call, the script's work,
and the Rust update of the retained tree. Layout and paint are not
included, and neither is a napi hop (gpuix's bench pays one tiny call per
event).

The modes compared:

- React-style: a render function re-runs on every state change and a Rust
  reconciler diffs the result, optionally skipping rows with `memo` or
  `memo_batch` (gpuix's Lua model).
- Solid-style: components run once, and signal changes update individual
  nodes in place. In Lua this is gpuix's own `solid.lua`. In JS it is real
  SolidJS (1.9, later 2.0) through its universal renderer.

### The workload

N rows, each a styled `div` with three `text` children: the row number, a
fixed sentence, and "selected" or nothing. A click on the root does one of
two things:

- `select` moves the selection to the next row, which changes two rows.
- `reverse` reverses the row order, which moves every row and changes none.

The modes:

| Mode | Model | What a click does |
|------|-------|-------------------|
| `full` | React | Re-renders every row; the reconciler diffs all of them. |
| `memo` | React | Re-renders the root; one `memo` per row skips unchanged rows. |
| `batch` | React | As `memo`, with one `memo_batch` call for all rows. |
| `solid` | Solid | Signals update nodes in place. Every row reads the selection, so a selection change re-runs every row's effect. |
| `selector` | Solid | As `solid`, but rows read the selection through a keyed selector, so only the two affected rows re-run. |

`solid.For` in gpuix renders into a slot `div` of its own, so Solid-style
trees have one extra `div` between the root and the rows. The SolidJS
workload adds the same `div`; the parity tests compare Solid trees with
React trees with that `div` removed.

gpuix's `solid.lua` has no `create_selector`, so the Lua workload carries a
minimal one, and the SolidJS workload uses the same helper rather than
SolidJS's `createSelector`. SolidJS's version compares every subscribed key
on each change: about 2.5 ms per selection change at 10,000 rows in QuickJS,
against about 10 µs for the keyed helper. The keyed helper only supports
plain equality, which is all the workload needs.

### Results

`ROUNDS=5 scripts/bench-all.sh`, 10,000 rows, 50 updates per round, on an
M3 Max with load averages of 8–16 from other work. Medians over rounds, in
milliseconds per update. "Commit" is the Rust-only part: the reconciler for
React modes, the flush for Solid modes.

| Mode | Scenario | Lua 5.4 | LuaJIT | QuickJS | QuickJS unfixed | Commit |
|------|----------|--------:|-------:|--------:|----------------:|-------:|
| `full` | select | 24.0 | 21.7 | 19.6 | 18.8 | 8.4–9.7 |
| `memo` | select | 4.3 | 4.2 | 4.1 | 4.1 | 1.3 |
| `batch` | select | 2.6 | 2.6 | 2.8 | 2.6 | 1.3 |
| `solid` | select | 10.1 | 6.2 | 12.8 | 12.3 | 0.0 |
| `selector` | select | 0.004 | 0.005 | 0.010 | 0.037 | 0.0 |
| `full` | reverse | 30.7 | 28.7 | 20.5 | 22.0 | 11.6–13.9 |
| `memo` | reverse | 5.9 | 6.1 | 5.9 | 5.8 | 3.1–3.3 |
| `batch` | reverse | 4.3 | 4.2 | 4.8 | 4.5 | 3.0–3.4 |
| `solid` | reverse | 7.3 | 3.5 | 14.0 | 76.1 | 0.6–1.0 |
| `selector` | reverse | 8.1 | 3.7 | 12.7 | 74.9 | 0.5–1.0 |

Mount, in milliseconds:

| Mode | Lua 5.4 | LuaJIT | QuickJS | QuickJS unfixed |
|------|--------:|-------:|--------:|----------------:|
| `full` | 30 | 26 | 21 | 21 |
| `memo` | 33 | 31 | 27 | 26 |
| `batch` | 32 | 31 | 25 | 25 |
| `solid` | 74 | 52 | 135 | 140 |
| `selector` | 83 | 73 | 182 | 249 |

What stands out:

- React-style, the engines are close. Once rows are memoized, most of the
  time is the Rust commit, which is the same for every engine. QuickJS is
  fastest at full re-renders and at mounting.
- Solid-style with the keyed selector, a selection change costs 4–10 µs
  instead of 2.6 ms for the best React mode: only two rows do any work, and
  the Rust side has nothing left to diff.
- Without the selector, every row re-runs on each selection change, and
  Solid-style is slower than memoized React-style in every engine.
- Reversing 10,000 rows is a different story: SolidJS in QuickJS takes
  13–14 ms, against 4–5 ms for `batch` and 3.5 ms for gpuix's `solid.lua` in
  LuaJIT. Without the Map hash fix it takes 75 ms. Most of that is SolidJS's
  own work in an interpreter, not calls into Rust (see "Batching inserts
  doesn't pay off in QuickJS").
- Solid-style mounting costs 2–7 times React-style mounting, because it
  builds a reactive graph next to the tree. SolidJS in QuickJS mounts about
  twice as slowly as `solid.lua`, partly because it creates more host nodes
  (see "The Solid hosts").
- The "commit" column is near zero for Solid modes: node changes go straight
  to the retained tree, and only child order waits for the flush.

### Real React and Solid 2.0

Measured for Phase 0 of `embedded-quickjs-runtime.md`: could a QuickJS app run
the real `@gpuix/react`, and what does Solid 2.0 cost next to 1.x?

`WARMUP=20 ITERATIONS=30 bench all all all`, 10,000 rows, one run on the M3
Max with load averages of 22–35 from other work, so treat single numbers as
±10%. Medians in milliseconds per update. Rerunning `full` select at 2,500,
5,000 and 10,000 rows gave 59, 120 and 229 ms.

| Mode | Scenario | QuickJS, Rust reconciler | React, json | React, direct | Bun, React, no host |
|------|----------|----:|----:|----:|----:|
| `full` | select | 14.8 | 224 | 229 | 14 |
| `memo` | select | 3.7 | 34 | 33 | |
| `full` | reverse | 20.0 | 2,376 | 2,380 | 141 |
| `memo` | reverse | 6.5 | 2,419 | 2,655 | |

| Mode | Scenario | SolidJS 1.x | Solid 2.0 |
|------|----------|----:|----:|
| `solid` | select | 12.7 | 21.7 |
| `selector` | select | 0.010 | 0.037 |
| `solid` | reverse | 15.0 | 14.6 |
| `selector` | reverse | 14.3 | 14.4 |

Mount: React 340–520 ms against 20–40 ms for the Rust reconciler; Solid 2.0
185–245 ms against 135–205 ms for 1.x.

What stands out:

- Real React in QuickJS is 9–15 times slower than the Rust reconciler on
  the same engine: 224 ms per selection change with every row re-rendering,
  34 ms with `React.memo` rows. The Rust side does almost nothing; the time
  is React's render and commit in an interpreter.
- Reversing 10,000 rows takes 2.4 s, memoized or not. That is React's own
  algorithm: the time roughly triples each time the rows double (66, 202,
  675, 2,386 ms at 1,250–10,000 rows), and the same bundle under Bun, with a
  host that does nothing, shows the same curve (12, 44, 141 ms at
  2,500–10,000). QuickJS runs React about 16 times slower than Bun's JIT,
  for both select and reverse.
- The two transports cost the same. Each commit's JSON batch is small next
  to React's own work, so `applyBatch` can stay as the embedded transport.
- Solid 2.0 matches 1.x on reverse and with the selector, and is slower
  where every row re-runs (22 against 13 ms). Its `selector` mode uses a
  projection, 2.0's replacement for `createSelector`; 37 µs is still nothing
  next to any React mode.
- Solid 2.0 renders `""` as no text node, as React does; 1.x renders an
  empty one. And a removed row in 2.0 stays reachable from a cycle in
  Solid's disposed reactive graph until the collector runs, where 1.x lets
  reference counting free it at once. The tests record both.

### quickjs-ng's Map hashing

SolidJS's `For` keys a `Map` by the list's items (here small integers), and
its array reconciler keys one by node objects. In quickjs-ng both degrade
towards a linear scan. `map_hash_key` multiplies the key by a constant,
which moves the variation into the high bits, then takes the bucket from the
low bits with `& (hash_size - 1)`. A small integer stored as a double has
zero low bits, and an object pointer has constant low bits from alignment,
so most keys land in a few buckets.

`packages/native/vendor/map-hash-repro.js` shows it on its own (M3 Max,
machine under load, nanoseconds per `set` or `get`):

| Keys | 1,000 | 10,000 | 100,000 |
|------|------:|-------:|--------:|
| Integers, unfixed | 5,583 | 5,417 | 1,125 |
| Objects, unfixed | 641 | 1,233 | 1,170 |
| Strings, unfixed | 256 | 216 | 500 |
| Integers, fixed | 137 | 155 | 258 |
| Objects, fixed | 180 | 297 | 224 |
| Strings, fixed | 129 | 160 | 224 |

The fix mixes the hash's high bits into its low bits (the MurmurHash3
finalizer) before the bucket is taken. Every Map and Set operation goes
through `map_hash_key`, so lookups stay consistent. gpuix vendors it in `packages/native/vendor/rquickjs-sys` behind the
`map-hash-mix` feature, which the `quickjs` feature turns on;
`packages/native/vendor/rquickjs-sys-0.14.0-map-hash-mix.diff` is the
complete change. Upstream quickjs-ng has the same code today. The
benchmark reports QuickJS with and without the fix.

#### The raw QuickJS path

rquickjs gives every `Value` its own reference to the context, so each value
the binding reads costs a `JS_DupContext`/`JS_FreeContext` pair of FFI calls.
Reading prop names through the safe API also allocates a `String` per key.
`src/js/raw.rs` reads props as interned atoms and reads integer tokens straight
from the value tag. With assertions still on, that took about 10–15% off full
renders (fastest about 24 → 21 ms) and changed little elsewhere. The module
documents its safety rules inline and is covered by the edge-case test in
`tests/parity.rs`.

rquickjs also compiles quickjs-ng with its C `assert`s on, unless the
`disable-assertions` feature is set. gpuix's `quickjs` feature sets it.

#### The Solid hosts

The direct host (`src/core/direct.rs`) keeps each node's parent and sibling
links, so a remove is constant time, and an insert or move costs only a
walk up the parent's ancestors, which rejects moving a node into its own
subtree. Element props go straight to the retained tree. Child order, and
text nodes folded into their `text` element, are written once per event at
the flush, because rebuilding a long child list on every insert would make
mounting quadratic. The flush is the "Rust-only commit" for Solid modes. It
first detaches every child that changes parent, then writes the new child
lists: written one by one, a parent and child that swapped places would
briefly be inside each other.

#### Batching inserts doesn't pay off in QuickJS

SolidJS inserts and moves one node at a time, so reversing 10,000 rows makes
about 10,000 `ui.insert` calls. Sending each changed parent's child list once
per update instead seemed likely to help. It didn't. Timed in scratch copies
(medians of four interleaved runs at a load average of about 12):

| SolidJS reverse | Event handler | Building lists (JS) | Rust batch call | Total |
|-----------------|--------------:|--------------------:|----------------:|------:|
| No structural Rust calls (timing only; the tree is wrong) | 10.7 ms | – | – | 10.7 ms |
| One `ui.insert` per insert | 13.0 ms | – | – | 13.0 ms |
| Batched per update | 13.2 ms | 1.0 ms | 1.25 ms | 15.5 ms |

The 10,000 calls cost about 2.3 ms, around 200 ns each, most of it
rquickjs's safe argument conversion. Tracking changed parents in JS instead
costs about as much, and the batch then walks every child twice more, once
in JS and once in Rust. Most of the time is SolidJS itself: with no
structural calls at all, it still takes about 10.7 ms. Batching would pay off
where each call is expensive, such as a napi or WASM boundary.

SolidJS walks the tree it builds (parent, first child, next sibling) and
compares nodes by identity, so `renderer.js` keeps one JS object per host
node with those links, and tree reads never cross into Rust. When a node
object is garbage collected, a `FinalizationRegistry` callback frees the
Rust node. An attached node can't be collected while its parent is alive,
because the parent's links reference it.

Event handlers stay on those JS objects. Rust only records which events a
node listens for, and a click goes to one JS dispatcher that finds the node
through a `WeakRef`. Holding the handlers in Rust instead would keep alive
any node whose handler refers to the node itself, since the collector can't
see references held from Rust. Reference counting frees removed leaves,
like replaced text nodes, at once. A removed subtree is a cycle through its
parent and child links, as is a node whose handler refers to it, so those
wait for the collector. Either way the Rust node is released at the next
settle, when the registry's callbacks run.

The Lua Solid host follows gpuix's rules instead, because `solid.lua`
expects them: nodes are created with their children, `set_children` destroys
the children it drops, and nodes still unattached when an event finishes are
destroyed. gpuix queues these calls and applies them at its next flush; here
they apply at once.

The two libraries don't do the same work for the same workload, and the
numbers include that:

- Lua's `solid.text` calls `set_text` every time its effect runs, even with
  an unchanged value; SolidJS skips unchanged values before calling the host.
- When a text value changes from `""`, SolidJS replaces the text node
  rather than changing it, so each newly selected row gets a new text node
  and the old one is dropped. Lua keeps one node and changes its text.
- SolidJS creates a separate text node inside each `text` element, so it
  mounts more host nodes, each registered with the `FinalizationRegistry`.
  Its mount also includes evaluating the 40 KB bundle.
- Lua's `solid.For` gives every row an item signal and a position signal,
  and sets both for every row whenever the list changes. SolidJS's `For`
  passes the item as a plain value and only tracks positions when the row
  function takes an index argument.
