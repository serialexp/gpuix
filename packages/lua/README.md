# GPUIX Lua components

These modules are bundled into `gpuix-lua` and loaded through ordinary Lua
imports:

```lua
local Select = require("gpuix.select")
local Combobox = require("gpuix.combobox")
local Tooltip = require("gpuix.tooltip")
local Drawer = require("gpuix.drawer")
local DockLayout = require("gpuix.dock_layout")
```

The package currently provides `gpuix.button`, `gpuix.checkbox`, `gpuix.radio_group`,
`gpuix.select`, `gpuix.combobox`, `gpuix.tooltip`, `gpuix.drawer`, and
`gpuix.dock_layout`. They are headless LuaX components built from GPUIX host
elements, so applications own their styling.

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
Drawer follows Zed's dock-panel model: it occupies layout space on the left,
right, or bottom; supports controlled or uncontrolled open and size state; and
has a focusable resize handle. Drag the handle, use the directional arrow keys
to resize by 8 pixels (32 with Shift), or press Home/double-click to restore
`defaultSize`.

`DockLayout` adds Zed's surrounding dock chrome. Its bottom status bar renders
one button per panel; clicking toggles the panel, while right-clicking opens a
menu that moves it to the left, right, or bottom dock. One panel can be open on
each edge. Panel shells remain mounted but GPUI-invisible while closed, so hooks
and memoized rows survive toggling and re-docking.

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
local Drawer = require("gpuix.drawer")
local DockLayout = require("gpuix.dock_layout")

<Button onClick={save}><text>Save</text></Button>
<Checkbox checked={enabled} onCheckedChange={set_enabled} label="Enabled" />
<RadioGroup value={density} onValueChange={set_density} options={{
    { value = "roomy", label = "Roomy" },
    { value = "compact", label = "Compact" },
}} />
<Select value={runtime} onValueChange={set_runtime} options={runtimes} />
<Combobox value={framework} onValueChange={set_framework} items={frameworks} />
<Tooltip content="Runs without JavaScript"><text>LuaX</text></Tooltip>
<Drawer
    side="right"
    open={activity_open}
    defaultSize={320}
    minSize={200}
    maxSize={560}
    onOpenChange={set_activity_open}
    onSizeChange={save_activity_size}
    renderContent={function(state)
        return <ActivityPanel close={state.close} />
    end}
/>
<DockLayout panels={{
    {
        id = "activity",
        label = "Activity",
        buttonLabel = "ACT",
        position = "right",
        defaultOpen = true,
        defaultSize = 320,
        render = function(state)
            return <ActivityPanel close={state.close} side={state.position} />
        end,
    },
}}>
    <Workspace />
</DockLayout>
```

Select exposes `triggerStyle`, `contentStyle`, `itemStyle`, `renderValue`, and
`renderItem`. Combobox exposes `inputStyle`, `contentStyle`, `itemStyle`,
`emptyStyle`, and `renderItem`. A style prop may be a table or a function that
receives that part's state.

Drawer accepts `side`, `open`, `defaultOpen`, `size`, `defaultSize`, `minSize`,
`maxSize`, `resizable`, `resizeHandleSize`, and `resizeTabIndex`. Its callbacks
are `onOpenChange`, `onSizeChange`, `onResizeStart`, and `onResizeEnd`. Use
`style`, `contentStyle`, and `resizeHandleStyle` to style its surfaces.
`renderContent(state)` is lazy and receives `open`, `side`, `size`, `close`,
`setOpen`, and `setSize`; `renderTrigger(state)` can render a replacement while
an uncontrolled drawer is closed. Plain children are also accepted.

`DockLayout` accepts `panels` plus `style`, `workspaceStyle`, `contentStyle`,
`statusBarStyle`, `buttonStyle`, `activeButtonStyle`, `menuStyle`, and
`menuItemStyle`. Its callbacks are `onPanelToggle`, `onPanelPositionChange`,
and `onPanelSizeChange`. Their payloads are `{ id, open, position }`,
`{ id, position, previousPosition }`, and `{ id, size }`, respectively. Each
panel requires `id` and may set `label`, `buttonLabel`,
`position`, `defaultOpen`, `defaultSize`, `minSize`, `maxSize`, `resizable`,
`resizeHandleSize`, `resizeTabIndex`, `hideButton`, `tabIndex`, `style`,
`contentStyle`, `resizeHandleStyle`, `buttonStyle`, `activeButtonStyle`,
`onResizeStart`, `onResizeEnd`, `render(state)`, and `renderButton(state)`.
`render` receives `id`, `open`, `position`, `size`, `close`, and `toggle`;
`renderButton` receives `id`, `active`, and `position`. A right-docked button
group is displayed in reverse declaration order, matching Zed.

Lua applications can keep SVG assets in an ordinary module and pass the raw
source to GPUIX's native monochrome renderer. The workspace's `icons.lua`
imports a Phosphor Pulse asset this way and uses it from `renderButton`:

```lua
local icons = require("icons")

renderButton = function(state)
    return <svg source={icons.activity} style={{
        width = 16,
        height = 16,
        color = state.active and "#fafafa" or "#a1a1aa",
    }} />
end
```

Phosphor's raw files use `fill="currentColor"`; change that to `fill="#000"`
when copying an asset into a Lua string. GPUIX then applies the desired tint
from `style.color`.

The Lua runtime has no timer scheduler or host refs yet. Tooltip delay windows
and ref-based focus restoration therefore remain React-only behavior for now.
