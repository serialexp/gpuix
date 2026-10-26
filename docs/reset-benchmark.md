# Interpreter reset timing

Measured on 2026-10-23 on an Apple M3 Max (arm64 macOS), with a release gpx
build, vendored Lua 5.4 and embedded QuickJS development-mode React. These
are fresh process launches with ordinary OS caches, not cold-machine starts.

```sh
LAUNCHES=7 ITERATIONS=20 bun scripts/bench-reset.ts \
  examples/solid-counter.luax \
  examples/solid-workspace/main.luax \
  examples/counter.tsx
```

There were seven fresh launches and 140 interpreter resets per example.
Every reset kept the same native window and recreated the interpreter,
retained tree and GPUI root. Node counts were checked across resets.

| App | Retained nodes | Fresh process to first scene, median | Reset to first scene, median | Reset p95 |
| --- | ---: | ---: | ---: | ---: |
| Solid-like Lua counter | 27 | 235.6 ms | 3.07 ms | 3.64 ms |
| Solid-like Lua workspace | 105 | 239.7 ms | 6.42 ms | 7.83 ms |
| Embedded React counter | 16 | 251.2 ms | 16.78 ms | 18.92 ms |

The fresh-process endpoint is arrival of a stderr marker emitted immediately
after the initial CPU scene build. The driver starts its clock before spawning
gpx. In-process reset clocks stop after rebuilding that same CPU scene.
Pipe delivery adds a little overhead to the fresh-process measurement.

## Where the reset time goes

| Phase, median | Lua counter | Lua workspace | Embedded React counter |
| --- | ---: | ---: | ---: |
| Load/evaluate interpreter and source | 2.51 ms | 3.05 ms | 15.67 ms |
| Lua mount/reconciliation | 0.14 ms | 1.17 ms | included above |
| Dispose previous interpreter | 0.14 ms | 0.36 ms | 0.88 ms |
| Replace view and release old retained state | 0.15 ms | 0.96 ms | 0.11 ms |
| GPUI layout/paint scene build | 0.14 ms | 0.89 ms | 0.08 ms |

Medians of individual phases need not sum exactly to the median total.
JavaScript loading includes framework bundle evaluation, source compilation,
initial rendering and its mutation commit. Lua loading and mounting are
separate phases. Native initialization outside opening the window accounts
for roughly 155–158 ms of each fresh launch; window creation adds roughly
60–66 ms. Process loading and notification overhead account for most of the
remaining difference from the measured internal initial-scene total.

For these apps, retaining native resources saves roughly 220–235 ms per
scenario. Lua reconstruction is inexpensive. Embedded React reconstruction is
mostly interpreter/framework/module startup, not GPUI drawing. This supports
using fresh interpreters in a shared native test host rather than snapshotting
the retained tree or launching another native process per scenario.

## Scope and caveats

The harness measures only the first initially open window. It does not attach
the normal event, HTTP, tray or source-watcher loop, and does not wait for
images or other asynchronous application initialization. JavaScript immediate
startup timers are processed as in the normal launcher. The measured drawing
is CPU layout/paint scene construction, not GPU completion, scanout, screenshot
readback, or assertion polling. Thus these are reset-building-block timings,
not a claim that a full scenario isolation implementation already exists.

These examples have small trees. Bigger trees, expensive module initialization,
asynchronous startup, extra windows and external resources can change the
trade-off. A suite implementation must still rebind host callbacks and cancel
old asynchronous work, dispose old native subscriptions and define what happens
to extra windows and native state. No Lua/JS state snapshot is used here.

The driver writes full per-launch/per-reset samples to the ignored file
`packages/react/screenshots/reset-bench/results.json`. Rerunning it replaces
that file. The native measurement can also run directly with
`gpx bench-reset APP --iterations 20`.
