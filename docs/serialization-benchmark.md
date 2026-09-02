---
title: applyBatch serialization benchmark
description: >
  What the mutation wire format and the Rust data model actually cost, measured
  on the real ChatApp mutation queue. Covers why the codec is the smallest
  lever, why StyleDesc is 1392 bytes, and the compact tree that makes a
  million-element React tree affordable.
---

# applyBatch serialization benchmark

Every React commit turns into one `applyBatch(json)` call. This benchmark
measures what that costs on both sides of the FFI boundary, on the real
`ChatApp` queue rather than a synthetic one.

**Headline: swapping one serialized codec for another is the smallest lever.**
MessagePack buys 1.24x once bytes are inside Rust. Decoding straight into typed
ops and sharing styles by content bought **4.2x on parse-and-apply, 7.9x fewer
allocations, and 5.3x on tree memory**, with JSON unchanged and no change to
`apply_styles`.

That does not mean the end-to-end transport is free. The combined benchmark
added later measures JavaScript encoding, napi string extraction and copying,
Rust decode/apply, and return conversion in one timed operation. On the 10 000
turn fixture it measured **92.65 ms**, so a protocol that avoids serialization
and napi strings is a materially different proposition from replacing JSON
with another serialized codec.

That work has landed. What follows is how it was measured, what was rejected,
and what is left.

## What shipped

Measured end to end on the real path, `apply_batch_to_tree`, before and after:

| | before | after |
|---|---:|---:|
| parse and apply | 127.1 ms | **30.1 ms** |
| heap churn | 900.5 MB | **104.0 MB** |
| allocations | 1 476 196 | **186 090** |
| retained tree | 224.5 MB | **42.6 MB** |
| bytes per element | 3116 B | **592 B** |

Four changes, in the order they landed:

1. `BatchOp::SetStyle` stopped holding a `StyleDesc` inline. A `Vec<BatchOp>` is
   as wide as its widest variant, so 221 764 ops reserved 312 MB up front.
   Measured on its own: **−205 MB churn**
2. The `serde_json::Value` tree is gone. The batch deserializes straight from
   its bytes into typed ops through a `SeqAccess` visitor, and strings borrow
   out of the input
3. `RetainedTree::elements` uses `FxHashMap`. It is probed once per ancestor hop
   in `mark_changed` and twice per child per frame in `build_virtual_list`
4. Styles are hash-consed by raw payload and shared as `Arc<StyleDesc>`, swept
   after each batch by `Arc::strong_count`
5. `commitUpdate` now compares distinct style objects structurally instead of
   resending every style on every host update. Style props follow React's
   immutable-props model; mutating one object in place is not an update

Rejected, with reasons, in [what was not done](#deliberately-not-doing).

## Running it

Two halves. The JS half writes the fixture the Rust half reads, so both measure
the same bytes.

```bash
cd examples
TURNS=10000 SAFE_MDX=1 bun run bench:serde     # writes tmp/batch-fixture.json

cd ../packages/native
cargo run --release --example bench_serde
```

`examples/bench-serialization.ts` installs a stub `NativeRenderer` that owns
`applyBatch`, so `wrapWithBatching` hands it the exact tuples production sends.
Nothing about the fixture is invented.

```
ChatApp ► reconciler ► wrapWithBatching ► CaptureRenderer.applyBatch(json)
                                                 │
                                    ┌────────────┴────────────┐
                                    ▼                         ▼
                            JS codec bench          tmp/batch-fixture.json
                                                              │
                                                              ▼
                                                packages/native/examples/bench_serde.rs
```

The Rust half installs a counting `GlobalAlloc`, so "live heap" is the real
resident cost of the tree, not a `size_of` estimate. It also asserts that the
compact tree and the current tree hold the same parents, the same child order
and the same styled elements before reporting any saving.

The JS command also runs a combined boundary measurement through the native
`TestGpuixRenderer.applyBatch`. That call has the same napi `String` argument,
tree mutex, and `apply_batch_to_tree` implementation as the production
renderer. It reports three medians from the same process:

- `JSON.stringify(queue)` — JavaScript encoding only
- `applyBatch(pre-encoded)` — napi string extraction and copying, Rust
  decode/apply, and conversion of the return value to JavaScript
- `JSON.stringify → applyBatch` — the complete mutation transport path

The combined row deliberately excludes GPUI layout and paint. It reapplies the
same mount batch to the same retained tree, replacing the same element IDs on
every sample, so it measures a warm mutation boundary rather than native window
construction or a frame.

On the 10 000-turn fixture, five iterations on the benchmark machine produced:

| path | median |
|---|---:|
| `JSON.stringify(queue)` | 34.25 ms |
| `applyBatch(pre-encoded)` | 47.49 ms |
| `JSON.stringify → applyBatch` | **92.65 ms** |

The medians are independent and are not additive. In particular, the combined
path creates a 13.06 MB JavaScript string immediately before napi extracts it,
so allocation and GC pressure differ from repeatedly passing one pre-encoded
string. The combined row is the number to use for the current end-to-end path;
the other rows explain it, but must not be subtracted to claim a precise napi
cost.

## Workload matrix

The 10 000-turn chat is intentionally a wide-tree stress case, not a model of
every heavy React application. `examples/bench-workloads.tsx` runs the same
combined napi measurement over structurally different mounts and updates:

- the existing children-mode chat list
- a depth-8 ternary tree with varied styles
- 1 000 interactive dashboard cards with pointer, keyboard, and input events
- 2 000 media cards carrying image paths and raw SVG source
- one changed leaf inside 10 000 retained rows
- a broad theme update across 1 000 cards
- reversal of 5 000 keyed rows

```bash
cd examples
CHAT_TURNS=10000 ITERATIONS=5 bun run bench:workloads
```

One run on the benchmark machine after structural style comparison produced:

| workload | kind | ops | JSON | combined transport |
|---|---|---:|---:|---:|
| wide chat | mount | 221 764 | 13.06 MB | 101.01 ms |
| deep branches | mount | 68 889 | 2.50 MB | 13.58 ms |
| interactive dashboard | mount | 51 195 | 1.76 MB | 7.63 ms |
| media grid | mount | 50 003 | 2.26 MB | 9.21 ms |
| single leaf update | update | 2 | <0.01 MB | 0.01 ms |
| broad theme update | update | 1 155 | 0.08 MB | 0.34 ms |
| keyed reorder | update | 4 999 | 0.14 MB | 9.36 ms |

The operation mix matters as much as the total. The dashboard emits 3 026
event-listener operations. The media grid emits 6 000 custom props. Reversing
the keyed list remains expensive despite a 0.14 MB payload because Rust must
unlink and reattach 4 999 children. Before the style fix, changing one leaf in
the un-memoized 10 000-row fixture emitted 30 001 complete `setStyle`
operations plus one `setText`. It now emits one changed style and one text op;
the broad theme update drops from 7 028 styles to the 1 155 that differ, and a
pure reorder emits no styles.

### Images in the transport benchmark

An `<img>` does not send pixels through napi. Its `src` custom prop is a local
filesystem path, and GPUI loads, decodes, animates, uploads, and paints the image
later. The media workload therefore measures the path, `objectFit`, styles, and
tree mutations, not image decode or GPU cost.

Raw `<svg source>` is different: the complete SVG source string is a custom
prop and does cross the mutation boundary. The media workload repeats an inline
SVG icon on every card, so those bytes are included. A separate GPU benchmark
using real files and `renderer.flush()` is required to compare image decoding,
cache behavior, animation, upload, layout, and paint; none of those costs should
be attributed to the wire protocol.

## Does moving the host diff to Rust win?

The optimistic fixed-record prototype made comparison look almost free, so the
benchmark now includes a real second transport rather than extrapolating from
that inner loop:

1. React host mutations maintain an authoritative JavaScript host tree with
   O(1) sibling insertion and removal
2. The complete tree is serialized as compact JSON tuples and crosses the real
   `applySnapshot` napi entry point
3. Rust parses and validates the complete graph before mutation, interns every
   style, reconciles records by stable host ID, preserves unchanged revisions,
   removes missing IDs, and returns destroyed IDs
4. The production and test renderers invalidate GPUI exactly as `applyBatch`
   does

The mode is deliberately opt-in through
`createRoot(renderer, { transport: "snapshot" })`; mutations remain the
default. Both paths exclude GPUI layout and paint.

The reset-to-before transport medians from the same 10 000-turn run were:

| workload | mutation JSON | mutation combined | snapshot JSON | snapshot combined |
|---|---:|---:|---:|---:|
| wide chat mount | 13.06 MB | 101.01 ms | 9.81 MB | 89.84 ms |
| deep branch mount | 2.50 MB | 13.58 ms | 1.54 MB | 13.70 ms |
| interactive dashboard mount | 1.76 MB | 7.63 ms | 0.99 MB | 9.38 ms |
| media grid mount | 2.26 MB | 9.21 ms | 1.47 MB | 10.36 ms |
| single leaf update | <0.01 MB | 0.01 ms | 3.83 MB | 44.72 ms |
| broad theme update | 0.08 MB | 0.34 ms | 0.99 MB | 9.52 ms |
| keyed reorder | 0.14 MB | 9.36 ms | 1.42 MB | 11.53 ms |

Those rows time only snapshot serialization plus the napi/Rust endpoint. The
benchmark also renders through the actual native renderer and reports the full
React commit wall, including component execution, Fiber, host bookkeeping,
serialization, napi, and Rust apply. Three-sample medians produced:

| workload | mutation commit | snapshot commit |
|---|---:|---:|
| wide chat mount | **200.94 ms** | 219.13 ms |
| deep branch mount | **35.98 ms** | 37.06 ms |
| interactive dashboard mount | **21.43 ms** | 24.38 ms |
| media grid mount | 26.12 ms | **23.23 ms** |
| single leaf update | **45.20 ms** | 86.45 ms |
| broad theme update | **12.36 ms** | 25.48 ms |
| keyed reorder | **148.50 ms** | 150.36 ms |

So the answer is no as a general replacement. Rust record comparison remains
cheap, but full snapshot construction, JSON parsing, graph validation, style
resolution, and walking unchanged records usually cost more than the mutation
work they replace. Mounts are close enough for run-to-run variance to swap an
individual result, and the smaller wide-chat snapshot wins its isolated
transport row, but the complete wide-chat commit still loses. The leaf update
is decisive: two mutation tuples become a 60 001-node snapshot. Even the
favorable keyed reorder reaches only approximate parity after O(1) JavaScript
sibling bookkeeping.

`examples/bench-tree-diff.rs` remains as an explanatory microbenchmark. It
compares sorted 40-byte records and finds the two changed leaf records in about
0.15 ms, or 5 000 reordered records in about 0.05 ms. That proves comparison is
not the expensive part. It does not include producing the tree, crossing napi,
parsing properties, updating `RetainedTree`, or invalidating GPUI. The real
`applySnapshot` results include those costs and are the baseline to use.

An arena shared directly with an embedded language could change the result by
removing JSON and JavaScript snapshot construction. With React still producing
the host tree, however, the incremental mutation transport is the better
baseline.

## The fixture

10 000 turns with safe-mdx enabled. 221 764 ops, 72 010 elements, 13.06 MB of
JSON.

| op | count | JSON bytes | share |
|---|---:|---:|---:|
| `setStyle` | 59 320 | 6.37 MB | 49.6% |
| `createElement` | 72 010 | 2.10 MB | 16.4% |
| `appendChild` | 72 009 | 1.92 MB | 15.0% |
| `setCustomProp` | 6 142 | 1.70 MB | 13.2% |
| `setText` | 12 255 | 0.74 MB | 5.8% |

Two facts decide everything below.

**Styles are half the payload and there are only 90 of them.** 59 320 `setStyle`
ops carry 90 distinct style objects. The same ~1.2 KB object is serialized,
shipped and parsed hundreds of times.

**261 unique strings account for 564 987 occurrences.** 5.45 MB of string bytes
collapse to 0.01 MB once deduplicated.

## JS side: encode

| codec | encode | decode | wire bytes | vs JSON | resident while alive |
|---|---:|---:|---:|---:|---:|
| `JSON.stringify` | 23.96 ms | 18.96 ms | 13.05 MB | 1.00x | **26.1 MB** |
| JSON ► utf8 Buffer | 25.20 ms | 21.41 ms | 13.05 MB | 1.00x | 13.0 MB |
| msgpackr | 18.83 ms | 25.01 ms | 10.29 MB | 0.79x | 10.3 MB |
| msgpackr records | 16.85 ms | 11.15 ms | 6.96 MB | 0.53x | 7.0 MB |
| cbor-x | 18.05 ms | 36.38 ms | 10.30 MB | 0.79x | 10.3 MB |

`JSON.stringify` costs **two bytes of JS heap per wire byte**. JSC keeps a
string as latin1 only when every code unit fits in a byte, and the chat payload
contains box-drawing characters. Encoding into a `Buffer` costs one byte per
wire byte, and that byte lives outside the JS heap where the GC never has to
walk it.

That is the one unambiguous JS-side memory win, and it does not require
changing the codec.

## JS side: protocol

Interning is not a codec feature. It changes what JS sends.

**Protocol A** replaces a repeated style object with `["setStyleRef", id, 7]`
plus one `["defineStyle", 7, {…}]` the first time that style is seen.
**Protocol B** adds a string table for every value that repeats.

| variant | JSON bytes | vs today | msgpackr records | vs today |
|---|---:|---:|---:|---:|
| today | 13.05 MB | 1.00x | 6.96 MB | 0.53x |
| A — interned styles | 8.10 MB | 0.62x | 6.05 MB | 0.46x |
| B — A + string table | 6.78 MB | 0.52x | **4.32 MB** | **0.33x** |

Protocol A alone also halves encode time, from 23.96 ms to 15.55 ms, because
`JSON.stringify` no longer walks 59 320 style objects.

## Rust side: decode

This is where the money is. Median of 5 runs, counting allocator.

| decode path | median | vs today | heap churn | allocations |
|---|---:|---:|---:|---:|
| **today** — `Vec<Value>` + clone + `from_value` | 63.7 ms | 1.00x | 206.8 MB | 1 755 376 |
| `Vec<Value>`, no clone | 56.4 ms | 1.13x | 150.9 MB | 1 294 094 |
| typed JSON ► `StyleDesc` | 71.1 ms | **0.89x** | 374.5 MB | 140 702 |
| typed JSON ► `CompactStyle` | 26.0 ms | 2.45x | 36.0 MB | 100 358 |
| typed msgpack ► `CompactStyle` | 19.6 ms | 3.24x | 42.3 MB | 100 356 |
| typed JSON ► `Box<StyleDesc>`, interned | **13.4 ms** | **4.74x** | 32.2 MB | 41 330 |
| typed JSON ► `CompactStyle`, interned | 14.8 ms | 4.30x | 32.1 MB | 41 115 |
| typed msgpack ► `CompactStyle`, interned | 10.8 ms | 5.91x | 38.4 MB | 41 113 |

**Once styles are interned, `CompactStyle` stops paying for itself.** Only 90 of
221 764 ops parse a style, so the 1392-byte struct is built 90 times and its
size no longer matters. `Box<StyleDesc>` keeps every named field, keeps
`apply_styles` untouched, and is marginally **faster**. Intern first; only
consider a compact style if a profile still points at style parsing.

### Today's path allocates the same data four times

`renderer.rs` takes `json: String`, so napi copies and UTF-8 validates the whole
payload before parsing starts. Then:

```
Rust String
   │
   ▼ serde_json::from_str            a String per key AND per value
Vec<serde_json::Value>
   │
   ▼ batch_payload -> value.clone()  a deep clone of the whole style map
serde_json::Value
   │
   ▼ serde_json::from_value          a String per StyleDesc field
StyleDesc
```

1.76 million allocations for 221 764 ops. Removing the `value.clone()` in
`batch_payload` alone is worth 1.15x and 460 000 allocations, costs nothing, and
needs no new dependency.

### A fat enum variant poisons the whole vector

The most useful negative result: going typed **without** fixing the style makes
things worse. `Op<StyleDesc>` is 1408 bytes, so a `Vec<Op<StyleDesc>>` of
221 764 ops reserves 312 MB regardless of how few ops carry a style. That path
churns 374.5 MB and is slower than the `serde_json::Value` tree it replaces.

`Op<CompactStyle>` is 104 bytes. Same code, 10x less churn.

## The data-oriented redesign

Everything in this section is a **phase two**. The recommended order above gets
5.4x on memory without any of it. Read this when 577 bytes per element is still
too much.

### `StyleDesc` is 1392 bytes for six properties

Around 80 `Option` fields, all inline, all allocated whether set or not. A real
element sets six. Three observations shrink it:

- GPUI consumes `Pixels(f32)`. Storing `f64` buys nothing
- a colour is 4 bytes once parsed, not a 24-byte `String` plus a heap block
- `display`, `position`, `alignItems` and friends are keyword enums, not text

So a style becomes a sorted list of 8-byte properties:

```rust
#[repr(C)]
struct StyleProp {
    key: u16,    // interned property name
    kind: u8,    // f32 | colour | keyword | bool | nested | percent | auto
    _pad: u8,
    bits: u32,   // f32 bits, or rgba8, or an id
}

struct CompactStyle { props: Box<[StyleProp]> }   // 16 B + 8 B per property
```

Parsing colours at ingest is a second win: paint stops calling `parse_color` on
every frame.

### `RetainedElement` costs 3102 bytes resident

`Option<StyleDesc>` is inline, so every element pays the full style size even
when it has none. Add a `String` element type, a `HashSet<String>` of event
names and a `HashMap<String, Value>` of custom props, and one node costs more
than a kilobyte before it holds any content. The `HashMap<u64, RetainedElement>`
that holds them rehashes 1.4 KB values as it grows.

The replacement is a fixed 44-byte record in a dense `Vec`, indexed by the id JS
already allocates as a counter. Children are a sibling list, so no node owns a
`Vec`. Everything variable-width moves to a side table that repeated values
share.

```rust
#[repr(C)]
struct CompactElement {
    parent: u32, first_child: u32, last_child: u32, next_sibling: u32,
    style: u32,           // index into a deduplicated style table
    content: u32, test_id: u32, custom_props: u32,
    subtree_revision: u32,
    events: u32,          // one bit per event type, was HashSet<String>
    kind: u16,            // div | text | input | …, was String
    custom_len: u16,
}
```

### Result

| tree | elements | live heap | per element | effort |
|---|---:|---:|---:|---|
| `RetainedTree` (today) | 72 010 | 223.4 MB | 3102 B | — |
| + `Box<StyleDesc>` | 72 010 | 124.6 MB | 1729 B | **one field** |
| + `Arc<StyleDesc>`, deduplicated | 72 010 | **41.6 MB** | **577 B** | one field plus an intern table |
| `CompactTree` | 72 010 | 9.2 MB | 127 B | rewrite every reader |

`size_of::<RetainedElement>()` was **1624 B** and is **248 B** with the style
behind a pointer. The `HashMap` holds the value inline, so that one field
decides the size of the whole table. The benchmark's own replica prints 240 B
because it leaves out `search_revision`.

**5.4x of the available 24.4x costs one field change.** Everything reading
`Option<&StyleDesc>` keeps working through `Deref`, so `apply_styles` and every
custom element stay untouched. The remaining 4.5x needs the dense-`Vec` rewrite.

Extrapolated to the goal of virtualizing in Rust instead of React:

| elements | today | `Arc` deduplicated | `CompactTree` |
|---:|---:|---:|---:|
| 100 000 | 310 MB | 58 MB | 12.7 MB |
| 1 000 000 | **3.1 GB** | **577 MB** | **127 MB** |

## What codecs can and cannot do

No mainstream JS↔Rust codec deduplicates repeated string **values**.

| option | what it deduplicates | usable here |
|---|---|---|
| msgpackr `useRecords` | object **keys** per shape | yes, but keys are not the cost |
| msgpackr `bundleStrings` | nothing; groups strings for one JS `toString()` | no, Rust cannot read the extension |
| msgpackr `structuredClone` | repeated object **references** | no, React makes a fresh style object per element |
| Arrow dictionary arrays | string values, properly | columnar; a mutation stream is not a table |
| FlatBuffers `create_shared_string` | string values, properly | needs a schema and codegen for every op |

`msgpackr` with `useRecords: true` is also **not plain MessagePack**. `rmp_serde`
cannot read it; pnpm had to write a transcoder for the same reason. The 6.96 MB
row above is not a payload Rust can decode as-is.

So value deduplication has to happen in the protocol. Once styles are interned,
the codec choice is worth 1.3x and nothing else.

## Why styles are interned in Rust, not in the protocol

The obvious design is to make **JS** do it: send `["defineStyle", n, {…}]` once
and `["setStyleRef", id, n]` afterwards. It was measured and rejected.

It is not free on the JS side, which is the first surprise. To emit a reference
you must know the style's identity, and `commitUpdate` in `host-config.ts`
resends `newProps.style` on every commit. A JSX `style={{…}}` literal is a fresh
object every render, so a `WeakMap` misses on everything except hoisted
constants. That leaves a content hash, and the cheapest content hash of a JS
object is `JSON.stringify`. Measured with the interning **inside** the timed
region:

| path | JS time | wire bytes |
|---|---:|---:|
| `JSON.stringify(queue)` | 26.31 ms | 13.05 MB |
| intern, then stringify | 23.35 ms | 8.10 MB |

1.13x, not the 1.5x an earlier version of this document claimed by timing a
queue that arrived pre-interned.

The real objection is lifetime. `examples/timeline.tsx` writes
`left: clip.start * pxPerSecond` inline, so a drag produces a distinct style
every frame. A JS-owned table grows by one entry per frame and nothing releases
it, and on that same frame the protocol sends a definition **plus** a reference
where it sends one op today. It would make the interactive path slower and leaky
to make the cold mount path faster.

Interning in Rust has neither problem. `RetainedTree::intern_style` hashes the
raw payload, which is cheaper than building 80 `Option` fields, and
`sweep_styles` drops any entry whose `Arc::strong_count` is 1 after each batch.
A drag adds one entry per frame and releases it on the next.

The only thing given up is 4.95 MB of wire and 3 ms of JS.

## Deliberately not doing

| change | measured gain | why not |
|---|---|---|
| protocol interning in JS | 1.13x JS, 1.6x wire | leaks on a drag and makes the update path send two ops where it sends one. Rust interning gets the memory win without either |
| MessagePack | 13.4 ► 10.8 ms, **1.24x** | a new dependency and a new codec on both sides, for a quarter |
| `CompactStyle` | **none** once styles are interned | 80 property mappings and a rewrite of `apply_styles`, and it measured **slower** than `Box<StyleDesc>` |
| `CompactElement` dense tree | 41.6 ► 9.2 MB, 4.5x | every reader of `RetainedElement` changes, and it needs JS id recycling before its numbers mean anything |

## What is left

### Cheap, contained, unmeasured

Of the 592 bytes now left per element, none is the style:

- `element_type: String` ► a `u16` tag. Removes a heap block per element
- `events: HashSet<String>` ► a `u32` bitmask. `HashSet` is 48 B plus heap.
  Blocked on the custom-element trait signatures, which spell out `HashSet<String>`
- `HashMap<u64, RetainedElement>` ► a dense `Vec`. JS already allocates ids as a
  counter, so the hash buys nothing
- `Buffer` instead of `String` at the FFI. napi's `String` runs
  `napi_get_value_string_utf8` **twice**, once to measure and once to fill, so a
  13 MB batch is transcoded twice and allocated once more. `BufferSlice` is a
  pointer. This keeps JSON; it changes the container, not the format

### Probably the real 1M-element blocker

Neither of these is about serialization, and neither is measured:

- `RetainedTree::mark_changed` walks parent to root on every append, insert,
  style and text mutation. 72 009 `appendChild` ops at depth 10-15 is roughly a
  million map probes per mount
- `build_virtual_list` runs **every frame** and touches every child, not just
  the visible ones: two map passes plus a `child_ids.clone()`. At 72k rows that
  is ~144k probes per frame; at 1M it is ~2M probes plus ~8 MB of `Vec` clone

### Frame time

Everything measured here is **mount and memory**. At today's tree sizes a wheel
event still costs Taffy on the visible rows and is unchanged. That stops being
true at the 1M scale this work exists for, because `build_virtual_list` walks
every child per frame, so a smaller element does start to matter.

Also, at 1M elements one `applyBatch` is roughly a 180 MB JSON string held twice.
That is a separate wall, and no codec fixes it; a mount that large needs chunked
batches.

## Known gaps in the benchmark itself

Read these before quoting any phase-two number.

- **`CompactStyle` cannot deduplicate a style with a nested block.**
  `dedup_key` stores the *arena index* of a `hover` or `active`, and every
  occurrence gets a fresh index. That is why the Rust side reports 100 unique
  styles where the JS side reports 90: six shapes carry a nested object across
  sixteen ops, and `90 − 6 + 16 = 100`. `chat.tsx` barely uses `hover`; an app
  where every row has one would get **zero** style sharing from the prototype.
- `PropValue::visit_str` runs the CSS colour parser on every string value before
  it knows the property, and quantizes to 8 bits per channel, which is lossy for
  the oklch theme. The type has to come from the key, not from sniffing.
- `verify()` compares parents, child order, styled-ness and content presence. It
  does **not** compare element type, style content, text value, custom props or
  events, so a compact style that dropped half its properties would pass.
- `KEYS` is one namespace for element types, event names and style property
  names, and the compact tree does `1 << (id % 32)`, so event bits collide.
- The bench interners are thread-locals and are not reset between decode rows.
- `setCustomProp` payloads still decode to `serde_json::Value` everywhere.
  They are 1.70 MB of the fixture and hold its largest strings. They should stay
  a `Value`, because `build_element` reads `custom_props` every frame and
  reparsing there would be a frame-time regression, but the **clone** is gone.
- Escaped strings are rare here: exactly one of 312 198 value strings needed
  unescaping. MessagePack's guarantee that every string borrows is real and
  worth almost nothing on this payload.
