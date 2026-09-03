# GPUIX Lua components

These modules are bundled into `gpuix-lua` and loaded through ordinary Lua
imports:

```lua
local Select = require("gpuix.select")
local Combobox = require("gpuix.combobox")
local Tooltip = require("gpuix.tooltip")
```

The package currently provides `gpuix.button`, `gpuix.checkbox`, `gpuix.radio_group`,
`gpuix.select`, `gpuix.combobox`, and `gpuix.tooltip`. They are headless LuaX
components built from GPUIX host elements, so applications own their styling.

`gpuix.button` joins GPUI's native tab order by default. `Tab` and `Shift+Tab`
cycle through controls with a non-negative `tabIndex`; inputs and textareas join
that order automatically. Enter and Space activate a focused button through
GPUI without executing Lua keyboard handlers. The bundled default styles paint
a blue `focusVisible` ring during keyboard navigation; custom styles can supply
their own `focusVisible` nested style object. Radio-group arrow keys use
`gpuix.focus(host_handle)` to move native focus together with the selected value.

Select supports controlled and uncontrolled value/open state, disabled items,
anchored content, mouse highlighting, and Up/Down, Ctrl+P/Ctrl+N, Enter, Space,
and Escape keys. Combobox uses the native input and supports filtering,
single/multiple values, disabled items, empty content, and keyboard selection.
Tooltip opens from hover or focus and closes on pointer, blur, or Escape.

Unlike React's compound component API, each LuaX control is one component with
configuration props. LuaX renders child expressions eagerly, so a context
provider cannot currently affect nested component calls.

```lua
local Checkbox = require("gpuix.checkbox")
local Button = require("gpuix.button")
local RadioGroup = require("gpuix.radio_group")
local Select = require("gpuix.select")
local Combobox = require("gpuix.combobox")
local Tooltip = require("gpuix.tooltip")

<Button onClick={save}><text>Save</text></Button>
<Checkbox checked={enabled} onCheckedChange={set_enabled} label="Enabled" />
<RadioGroup value={density} onValueChange={set_density} options={{
    { value = "roomy", label = "Roomy" },
    { value = "compact", label = "Compact" },
}} />
<Select value={runtime} onValueChange={set_runtime} options={runtimes} />
<Combobox value={framework} onValueChange={set_framework} items={frameworks} />
<Tooltip content="Runs without JavaScript"><text>LuaX</text></Tooltip>
```

Select exposes `triggerStyle`, `contentStyle`, `itemStyle`, `renderValue`, and
`renderItem`. Combobox exposes `inputStyle`, `contentStyle`, `itemStyle`,
`emptyStyle`, and `renderItem`. A style prop may be a table or a function that
receives that part's state.

The Lua runtime has no timer scheduler or host refs yet. Tooltip delay windows
and ref-based focus restoration therefore remain React-only behavior for now.
