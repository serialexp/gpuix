# Mahina integration probe

2026-10-20. Experimental; not a replacement for `gpx`'s Lua runner.

## Runtime boundary

`serialexp/lune` is now Mahina. Its v0.1.2 release provides a static C ABI
library plus safe Rust crates (`mahina`, `mahina-sys`). GPUIX pins the release
bundle's source revision, `1b60a01f3af4013256d0b505ed8d281f80242fb5`.
The probe is opt-in through `mahina-experiment`; normal builds are unchanged.

The integration direction is to embed Mahina in the GPUIX Rust process, just
as GPUIX embeds Lua today. The stock `mahina` CLI does not acquire GPUIX by
requiring a Lua C module: Mahina does not provide general FFI. A Rust host
must register the UI operations and own the native event loop. There is no
reason to introduce a second process or an IPC protocol for this.

The existing host is not portable merely by switching a Cargo feature:
`lua_runtime.rs` uses `mlua` functions, tables, registry roots, userdata,
serialization and conversions. Mahina has its own corresponding concepts,
but does not implement the Lua ABI. Its safe values root heap objects and
keep the VM alive; never capture a VM in one of its registered callbacks.

The shortest useful first slice is the signal model:

```text
shared gpuix/solid.lua + gpuix/host.lua
       -> small guest host facade
       -> registered Rust functions through libmahina
       -> existing RetainedTree methods
```

The same guest source and tree-writing host methods run through `mlua` for
Lua 5.4/LuaJIT comparison. This measures Rust-to-guest calls, reactive work,
guest-to-Rust callbacks, string copies, child-array extraction and tree
invalidation together. No VM or host state is captured by the other in a
reference cycle: callback closures retain only the Rust tree.

## Probe coverage and limits

`packages/native/examples/bench_mahina.rs` embeds the real Lua signal core.
Mahina's parser defers table-call shorthand (`f { ... }`), so the two such
calls in the shared signal core use explicit parentheses (`f({ ... })`),
which also work in Lua 5.4 and LuaJIT. No engine-specific signal-core fork is
needed.
Its fixture mounts a root with sidebar and content, a keyed list, and five
retained nodes per row. Selection changes update just two row text nodes;
reversal preserves keyed nodes while replacing their parent's child array.
Assertions cover node counts, reactive update fan-out, keyed identity,
removed-row disposal and complete unmount. The example tests additionally
cover remount and recovery from an invalid host call.

This is deliberately headless: no layout, painting, window, native input,
image decoding, or screenshot is measured. It is a nested list workload,
not an entire application or a broad widget compatibility test. The tiny host
facade is not the production arena host, so compare engines within this
experiment, not its absolute numbers against old `bench-lua` results.

Default Mahina embedding enables tracing and native code generation where
supported. Native code generation is currently Linux x86-64 only. On this
Mac the trace IR interpreter and plain bytecode interpreter must be reported
separately; neither predicts a native-JIT result on Linux.

## Local measurements

2026-10-20, Apple M3 Max, macOS ARM64, 36 GiB RAM. Mahina v0.1.2's
published ReleaseFast archive, checked against the release SHA256; the Rust
bindings use the same source revision. Lua dependencies are pinned by
`packages/native/Cargo.lock`.

These are **indicative, not an isolated performance baseline**. Other builds
and background security scanning were active. Cold-mount variation was large;
do not use the small differences below to rank engines or set timing budgets.
There is no Mahina native-JIT measurement here.

Three fresh processes per engine/size, alternating engine order between runs.
Each process measures cold mount, then runs 200 untimed selection updates,
30 timed updates, and ten timed reversals. The update and reverse columns
are medians of the three process medians. All node-count, two-update,
key-preservation and disposal assertions passed in every run.

| Rows / host nodes | Engine | Cold mount median (ms) | Selection update median (µs) | Reverse median (ms) |
| --- | --- | ---: | ---: | ---: |
| 1,000 / 5,005 | Lua 5.4 | 27.6 | 11 | 1.52 |
| 1,000 / 5,005 | LuaJIT | 146.4 | 8 | 1.53 |
| 1,000 / 5,005 | Mahina bytecode | 22.0 | 16 | 3.92 |
| 1,000 / 5,005 | Mahina trace IR | 50.7 | 20 | 2.34 |
| 10,000 / 50,005 | Lua 5.4 | 226.3 | 9 | 15.29 |
| 10,000 / 50,005 | LuaJIT | 313.7 | 11 | 10.35 |
| 10,000 / 50,005 | Mahina bytecode | 308.2 | 19 | 33.45 |
| 10,000 / 50,005 | Mahina trace IR | 362.7 | 11 | 26.46 |

At 10,000 rows, cold-mount ranges were 198–711 ms (Lua 5.4), 239–542 ms
(LuaJIT), 275–614 ms (Mahina bytecode), and 336–460 ms (Mahina trace IR).
Worst observed selection samples across those runs were respectively
3.913, 0.979, 0.050 and 0.036 ms. The tiny medians are not a tail-latency
guarantee. We have not profiled those outliers or LuaJIT's cold-mount cost,
so they must not be attributed to a particular subsystem yet.

The final timed Rust executables used release optimization with final-example
LTO disabled to avoid repeated expensive full-GPUI link optimization during
iteration. Both engine builds used the same setting; the Mahina archive was
unchanged. Reproduce that configuration with:

```sh
cd packages/native
export MAHINA_PREBUILT_ROOT=/absolute/path/mahina-bundle-v0.1.2/lib
cargo rustc --release --features mahina-experiment --example bench_mahina -- -C lto=off
# Save/run that executable before building the other Lua backend.
target/release/examples/bench_mahina mahina 10000
target/release/examples/bench_mahina mahina-no-jit 10000
target/release/examples/bench_mahina lua 10000
cargo rustc --release --no-default-features --features test-support,luajit,mahina-experiment \
  --example bench_mahina -- -C lto=off
target/release/examples/bench_mahina lua 10000
```

The README's ordinary `cargo run --release` commands instead use the normal
full-LTO profile. Do not mix measurements from the two configurations.

The three lifecycle tests passed with both Lua feature configurations
(Mahina tracing on/off plus the selected Lua backend). For these checks,
`cargo rustc` used `--test -C lto=off` and the resulting test harness was run
directly; this changes test link cost, not which tests execute.

Takeaway: the signal library is portable and small reactive updates remain
cheap even with a large retained tree. This experiment does not establish a
Mahina speed advantage, nor does it measure a complete application. Repeat
on a quiet machine and on Linux x86-64 before judging native-JIT performance.

## Full integration path

1. Keep the Rust retained tree, GPUI window host and shared app registry.
   Embed Mahina on the GPUI foreground thread: its safe VM is `!Send` and
   `!Sync`, unlike the current `mlua` configuration. Do not hide it inside an
   unsafe `Send` wrapper; enqueue events to its owning thread instead.
2. Add a Mahina signal-host adapter, including style/prop conversion,
   validated live handles, ownership/disposal and guest event callbacks.
   Share engine-neutral host operations, not `mlua` value types. Preserve
   production slot-placement validation and transactional error behavior;
   the private trusted fixture is not a substitute for them.
3. Reuse the LuaX source transform, but provide module loading through the
   embedding API. Embedded Mahina intentionally has no filesystem authority
   by default. Inject a scoped loader rather than assuming CLI `require`
   can read application files.
4. Wire the adapter into the shared application/window loop and dev API;
   test click -> guest callback -> reactive mutation -> native paint. Port
   remaining store/window/services APIs and run bundled widget tests.
5. Only then consider the React-like hook/arena host, and benchmark both
   models with the same rendered application. Measure native-JIT behavior
   on Linux x86-64 before making a performance decision about that tier.

Mahina does not remove the need for a host boundary. The integration can
retain GPUIX's no-JSON design, but child arrays and string content still
have to cross into Rust-owned retained storage.
