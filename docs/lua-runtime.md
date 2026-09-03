# Embedded Lua runtime experiment

GPUIX now has an experimental embedded Lua renderer. Lua executes inside the
native runtime and reconciles directly into Rust's `RetainedTree`; it does not
serialize a host snapshot or send a mutation batch through napi after a state
update.

This is intentionally a runtime prototype, not a complete React replacement.
It includes a small native LuaX transform for React-like authoring syntax.

## Component model

A Lua source file returns one render function. Host helpers accept ordinary Lua
prop tables, immediately write a staged node into a Rust-owned render arena,
and return a packed integer handle. `gpuix.text(value)` consumes strings and
numbers into one native text node, event props are functions, and `key`
preserves identity across reorders. Rust never walks a returned Lua VNode table
tree.

```lua
local ui = gpuix

local function Counter()
    local count, set_count = ui.use_state(0)

    return ui.div {
        style = { display = "flex", gap = 12 },
        ui.text("Count: " .. count),
        ui.div {
            key = "increment",
            onClick = function()
                set_count(function(value) return value + 1 end)
            end,
            ui.text("Increment"),
        },
    }
end

return Counter
```

Static styles can be converted once and shared by native `Arc` identity on
every subsequent render:

```lua
local row_style = gpuix.style {
    display = "flex",
    gap = 12,
}

local function Row(label)
    return gpuix.div { style = row_style, gpuix.text(label) }
end
```

Host constructors return packed integer handles. Parent child arrays contain
only those handles; strings and numbers are consumed by `gpuix.text(value)` and
never share the child ABI. The token packs the current arena generation and
slot into one 64-bit Lua integer, so it requires no Lua allocation or GC object.
An event handler can pass a mounted handle to `gpuix.focus(handle)` to move
native keyboard focus without allocating a ref object. This is how composed
controls implement roving focus while keeping exactly one item in the tab order.

`gpuix.h(Component, props)` invokes a function component and is the target used
by the LuaX transform. Element helpers currently
cover `div`, `text`, `input`, `img`, `svg`, `code`, `markdown`, `diff`,
`anchored`, and `virtual_list`.

## LuaX syntax

`loadLuax(source)` transforms XML-like expressions into the specialized host
constructors and `gpuix.h(...)` component calls before Lua compiles the source.
Host tags start lowercase, component tags start uppercase, props use quoted
values or `{ Lua expressions }`, and function components receive one props table:

```lua
local function Button(props)
    return <div onClick={props.onClick}>
        <text>{props.label}</text>
    </div>
end

return function()
    local count, set_count = gpuix.use_state(0)
    return <div style={{ gap = 12 }}>
        <text>Count: {count}</text>
        <Button label="Increment" onClick={function()
            set_count(function(value) return value + 1 end)
        end} />
    </div>
end
```

LuaX is only syntax: it creates no VNode tables and adds nothing to the update
path. `<text>` interpolation compiles into one native text node. Expressions
under other host elements must already evaluate to a node handle, child-list
handle, `false`, or `nil`; this keeps ordinary numeric data distinct from packed
host handles. Nested LuaX is allowed inside `{ ... }` expressions.

The first transform deliberately omits fragments, spread props, and automatic
primitive expression children. Static literal children such as `<div>Hello</div>`
are wrapped in `gpuix.text(...)`, while dynamic text belongs in `<text>`.

## Modules

The native launcher supports standard cached Lua imports. Module names are
resolved from the entry file's directory, dots map to directories, and both
`.lua` and `.luax` modules are supported:

```lua
local theme = require("theme")
local Sidebar = require("components.sidebar")
```

For `components.sidebar`, the loader checks `components/sidebar.lua`,
`components/sidebar.luax`, and both `init` forms. Lua's normal
`package.loaded` cache still owns module identity, so module top-level code and
static `gpuix.style` declarations execute once. Module names cannot contain
path separators or escape the entry directory.

The native runtime also bundles a reusable LuaX component package. These
modules resolve before application-local files and require no JavaScript or
extra search path:

```lua
local Button = require("gpuix.button")
local Checkbox = require("gpuix.checkbox")
local RadioGroup = require("gpuix.radio_group")
local Select = require("gpuix.select")
local Combobox = require("gpuix.combobox")
local Tooltip = require("gpuix.tooltip")
```

Each control accepts controlled value props and matching change callbacks, or
`default*` props for internal state. Select and Combobox accept option tables,
state-aware style functions, disabled items, keyboard navigation, and anchored
content. See `packages/lua/README.md` for the current API and limitations.

`gpuix.button` has `tabIndex = 0` by default. GPUI owns the tab map, so `Tab`
and `Shift+Tab` move through LuaX buttons, form controls, inputs, and textareas
entirely in Rust. Set `tabIndex = -1` to keep click focus while
removing a control from sequential keyboard navigation. The bundled controls
use native `focusVisible` styles, so keyboard focus paints without a Lua state
update or event round trip.

Images use the same native element and props as React:

```lua
ui.img {
    src = "./avatar.png",
    alt = "Avatar",
    style = { width = 48, height = 48, borderRadius = 24 },
}
```

## Update path

```text
GPUI event
  -> in-process event channel (`gpuix-lua`)
     or the compatibility Node relay (`loadLua` / `loadLuax`)
  -> Lua handler inside Rust
  -> state setter or reducer dispatch marks the runtime dirty
  -> Lua render function
  -> host helpers populate a Rust-owned staging arena
  -> render returns one native node handle
  -> keyed reconciliation consumes the arena
  -> direct RetainedTree updates
  -> GPUI invalidation
```

The arena is reset before each render and committed only after the render
function returns a valid root. Each node handle packs the arena generation and
index into one integer; props, styles, child indexes, and event functions
already live in Rust. A retained `LuaNode` identity tree remains after commit
for keyed identity, memo ownership, and event handlers.

## Hooks

The runtime now exposes the basic stateful React primitives in Lua form:

```lua
local value, set_value = gpuix.use_state(0)
local state, dispatch = gpuix.use_reducer(reducer, initial_state)
local ref = gpuix.use_ref(initial_value)
local result = gpuix.use_memo(factory, { dependency })
local callback = gpuix.use_callback(function() ... end, { dependency })

gpuix.use_effect(function()
    local subscription = subscribe(value)
    return function() subscription:close() end
end, { value })
```

State setters accept either a value or an updater function. State setters and
reducer dispatch functions keep stable identity across renders; reducer
dispatch always invokes the latest reducer function. A ref is one stable table
with a mutable `current` field, and changing it does not schedule a render.

Hook dependency arrays use shallow Lua equality. Tables, functions, threads,
and userdata therefore compare by reference. Omitting the dependency array
recomputes or reruns on every render, while `{}` runs once until unmount or live
reload. `use_memo` and `use_callback` run during render. `use_effect` runs only
after the host tree commits, runs its previous cleanup before a changed effect,
and cleans up on component unmount and runtime shutdown. State updates from an
effect synchronously schedule another render. Effect and cleanup failures are
reported after commit and do not roll back an already-visible host tree.

Live reload preserves state and refs, but deliberately refreshes memo,
callback, and effect closures even when their dependency arrays are unchanged.
This prevents closures from continuing to execute the previous module version.

## Redux-style stores

`gpuix.create_store(name, reducer, initial_state)` creates a named store whose
state lives in Rust rather than in a reloaded Lua module. Recreating the same
name during live reload preserves its state and replaces its reducer.

```lua
local store = gpuix.create_store("counter", function(state, action)
    if action.type == "increment" then
        return { count = state.count + 1 }
    end
    return state
end, { count = 0 })

return function()
    local count = store.use_state(function(state) return state.count end)
    return <div onClick={function()
        store.dispatch({ type = "increment" })
    end}>
        <text>{count}</text>
    </div>
end
```

Store methods are bound functions and use dot syntax:

- `store.get_state()` returns the current state.
- `store.dispatch(action)` runs the latest reducer and returns the action.
- `store.subscribe(listener)` returns an idempotent unsubscribe function.
- `store.use_state(selector?, equality?)` subscribes the current component.

Without a selector, `use_state` returns the whole state. The default comparison
is shallow Lua equality, so tables compare by identity. A custom equality
function receives the previous and next selected values. Dispatch recomputes
every mounted selector before committing. A root render is scheduled only when
at least one selected value changed, and memo subtrees are invalidated only
when they contain a changed subscriber.

Reducers should follow Redux's immutable-update rule. Mutating and returning a
previously selected table keeps the same identity and therefore does not notify
that selector. Store names must be stable and unique within one runtime. Failed
live reloads restore the previous state, reducer, and listener set.

The Node addon remains available through `loadLua()` and `loadLuax()`, but a Lua
application does not need it. `gpuix-lua` reads the source in Rust, owns the
normal GPUI application loop, and routes GPUI events through an in-process
channel to Lua. The rendered tree never crosses napi or JavaScript.

Run the worked LuaX app without Bun, Node, or JavaScript:

```bash
cargo run --release --manifest-path packages/native/Cargo.toml \
  --bin gpuix-lua -- examples/luax-counter.luax
```

Use `--check` to parse, execute the initial render, and exit without opening a
window. `--title`, `--width`, and `--height` configure the native window.

## Memoized subtrees

`gpuix.memo(key, dependencies, render)` retains an unchanged host subtree and
lets Rust insert one cached arena handle instead of rebuilding or reconciling
that subtree.

```lua
rows[index] = ui.memo(index, selected == index, function()
    return Row(index, selected == index)
end)
```

Memo keys must be unique in one render. Dependencies may be scalar values or
tables containing strings, numbers, booleans, nil, and nested tables. The memo
callback should build a host subtree and should not call hooks directly. It may
render nested function components: their component metadata and hook state are
replayed on a cache hit.

For large homogeneous lists, `gpuix.memo_batch(keys, dependencies, render)`
does the same work in one Lua-to-Rust call and uses one shared render function:

```lua
local keys = {}
for index = 1, item_count do keys[index] = index end

return function()
    local dependencies = {}
    for index = 1, item_count do
        dependencies[index] = selected == index
    end
    local rows = gpuix.memo_batch(keys, dependencies, function(key, active, index)
        return Row(key, active)
    end)
    return gpuix.div { children = rows }
end
```

The key and dependency tables must be dense and have equal lengths. `render`
receives the key, dependency value, and one-based batch index, and runs only
for misses. This is intentionally a list-level primitive: the runtime cannot
infer that arbitrary neighboring component calls are independent, but a Lua
helper or future JSX transform can generate the batch without exposing its
bookkeeping to each application.

The batch result is one generation-tagged child-list handle, not a Lua table of
host handles. Its Rust-owned vector grows from the current runtime length and
is consumed directly by the parent, so a future LuaX `<For>` transform does not
need to know the list size at compile time. Ordinary Lua child tables continue
to work for dynamic or heterogeneous children.

## Preliminary benchmark

`bun run bench:lua` builds 10,000 keyed rows with nested text and changes the
selected row. Timings include one napi call carrying only the event, Lua
execution, native arena construction, reconciliation, and retained-tree
updates. They exclude GPUI layout and paint.

| Lua 5.4 implementation | Native nodes | Full tree | Memoized rows |
|---|---:|---:|---:|
| Returned Lua table tree | 70,001 | ~136 ms | ~12–16 ms |
| Rust arena + userdata handles | 70,001 | ~89 ms | ~16 ms |
| Packed handles + single-node text | 40,001 | ~36.5 ms | ~9.8 ms |
| Ordered reconciliation fast path | 40,001 | ~33.5 ms | ~8.3 ms |
| Typed memo values + reused buffers | 40,001 | ~33.5 ms | ~5–6 ms |
| Batched memo list | 40,001 | ~33.5 ms | ~1.8–2.0 ms |

An isolated 1,001-update LuaJIT run measured roughly 2.0 ms median for the same
memo batch. LuaJIT therefore does not currently beat Lua 5.4 on this
boundary-heavy cached update; most of the remaining work is in Rust. The dual
runner exists so this assumption is checked as the workload changes.

The equivalent LuaX memo-batch workload is effectively tied once warm: an
alternating 301-update Lua 5.4 run measured 1.70 ms for LuaX and 1.71 ms for
handwritten constructors. Warm compile + mount was 31.5 ms versus 30.7 ms, so
the one-time native transform cost was under a millisecond in that sample.

The latest path lowers initial mount to roughly 40–55 ms. It removes one Lua
GC allocation per host constructor and makes `gpuix.text(value)` one retained
node instead of a styled wrapper plus primitive leaf. Memoized updates still
walk all 10,000 memo declarations, but cached rows now return packed integer
handles rather than userdata.

When child identity and order are unchanged, reconciliation now zips the old
children with their staged handles directly. Keyed maps are allocated only
after an identity mismatch, preserving reorder semantics without charging the
common update path.

Memo keys and dependencies are now extracted into typed Rust values instead of
round-tripping through `serde_json`. Scalar dependencies allocate nothing, and
table dependencies are sorted structurally so insertion order does not affect
equality. The arena and memo tracking sets retain their capacity between
renders, while cached host identities use shared strings.

The batched path removes 9,999 Lua-to-Rust memo crossings and all per-row render
closures on a cache hit. It also keeps the resulting handle vector in Rust
instead of filling a 10,000-entry Lua table and immediately reading it back.
Numeric child identity and parent ownership checks use dense marker vectors
indexed by the render arena instead of hash sets. Only arbitrary host keys need
hash validation. The remaining profile is mostly Lua dependency-array
construction, memo lookup, string-key validation, and reconciliation;
dependency comparison itself is small. `memo_batch` walks each dense input
table with one stack-pinned sequence pass instead of setting up two
`mlua::Table::raw_get` calls per item, and parses values by reference. Raw Lua
values are cloned only for cache misses that call the render function.

Keeping the batch result in Rust reduced the previous roughly 3.4 ms median to
2.2–2.5 ms across repeated 101-update runs. Dense child validation then lowered
warm 201-update runs to roughly 2.1–2.2 ms. Stack-pinned table traversal lowers
the same runs again to roughly 1.8–2.0 ms. The latest profile contains no
meaningful child-list extraction cost or per-item `raw_get` wrapper overhead;
its largest remaining groups are memo lookup, retained-tree reconciliation, and
the remaining string-key validation.

These are indicative rather than an apples-to-apples React comparison. The
existing representative React leaf commit was about 45 ms on the same machine.
The useful result is that removing the returned Lua tree avoids substantial
conversion work, while React-style memoization remains important. Lua still
constructs transient prop and child tables for container helpers, but those are
consumed immediately rather than retained and traversed after render.

Run all three modes on both LuaJIT and Lua 5.4 with:

```bash
cd examples
bun run bench:lua
```

`ROWS`, `ITERATIONS`, `WARMUP`, `COOLDOWN_MS`, and `BETWEEN_MS` apply to the
whole suite. The runner snapshots both native binaries, restores the default
Lua 5.4 addon, waits thirty seconds after compilation and five seconds between
engine samples, then runs each mode on both snapshots. `SKIP_BUILD=1` reruns
existing snapshots without compiling. To benchmark only the currently built
engine and select one mode manually, use:

```bash
MEMO=1 bun run bench:lua:engine
BATCH=1 bun run bench:lua:engine
LUAX=1 BATCH=1 bun run bench:lua:engine
```

`LUAX=1` runs the same workload through `loadLuax`. Transformation happens
once during compile + mount; update timings execute the generated Lua and
should therefore match the handwritten form. With component-local hook
identity enabled, a 50-sample 10,000-row memo-batch run measured 2.20 ms for
handwritten Lua and 2.03 ms for LuaX on Lua 5.4.

## Current limitations

- Lua 5.4 is the default; LuaJIT is an optional native build feature.
- The hook set is `use_state`, `use_reducer`, `use_ref`, `use_memo`,
  `use_callback`, `use_effect`, and selector subscriptions through
  `store.use_state`. There is no context, layout effect, error boundary,
  asynchronous scheduler, transition, or concurrent rendering yet.
- Hook state belongs to a component instance. Instances are identified by their
  parent component plus an explicit `key`, or by component sibling position
  when no key exists. Hooks are then ordered only within that component.
- Every hook slot records the hook kind. State, reducer, and ref hooks also
  record the initializer's Lua type. Changing a component's hook count,
  swapping hook kinds, or swapping differently typed state-like hooks is
  rejected during a normal render. Live reload resets only the affected
  component and preserves state elsewhere. The initializer value itself is not
  identity, so changing `use_state(0)` to `use_state(1)` preserves state.
- Swapping two same-kind, same-type hooks is still ambiguous. Detecting that
  requires compiler-generated call-site IDs.
- Lua execution is desktop-only; the browser/WASM renderer has no embedded Lua.
- The napi renderer still relays events through Node by design; `gpuix-lua`
  bypasses the Node renderer entirely.
- LuaX currently covers the small syntax subset above rather than the complete
  JSX grammar.

The worked app is `examples/lua-counter.lua`, launched by
`bun run counter:lua` from `examples`.

The LuaX source is `examples/luax-counter.luax`. The native command above is the
normal launcher; `bun run counter:luax` remains available for exercising the
Node addon integration.

`examples/luax-workspace/main.luax` is the broader multi-file example. It uses
imported Lua and LuaX modules, nested function components, root and
component-local state, a reload-safe Redux-style store, native input, SVG,
Markdown, highlighted code, unified diffs, images, anchored layers, a virtual
list, the bundled form controls, and `memo_batch`:

```bash
cargo run --release --manifest-path packages/native/Cargo.toml \
  --bin gpuix-lua -- examples/luax-workspace/main.luax
```

Pass `--watch` to poll the entry directory for `.lua` and `.luax` changes.
Reload executes the entry and its imported modules again in the existing Lua
VM, then reconciles against the retained native tree. Hook state is preserved
when hook signatures are compatible. A signature change accepts the reload
with fresh state only for the affected component; syntax and render errors
leave the previous view running. The root `just start` recipe enables this mode
for the workspace example.
