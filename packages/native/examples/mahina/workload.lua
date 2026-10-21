local solid = require("gpuix.solid")
local cleanups = {}
gpuix = {}

function gpuix.text(value)
    local handle = __gpx_create("text")
    __gpx_text(handle, tostring(value))
    solid.on_cleanup(function() __gpx_destroy(handle) end)
    return handle
end

function gpuix.set_text(handle, value)
    __gpx_text(handle, tostring(value))
end

function gpuix.set_children(handle, children)
    __gpx_children(handle, children)
end

function gpuix.set_child(handle, child)
    gpuix.set_children(handle, child and { child } or {})
end

function gpuix.div(props)
    local handle = __gpx_create("div")
    solid.on_cleanup(function()
        __gpx_destroy(handle)
    end)
    gpuix.set_children(handle, props.children or props)
    return handle
end

function gpuix.on_root_cleanup(callback)
    cleanups[#cleanups + 1] = callback
end

local set_selected, set_items, items, selected, row_handles

function mount(count)
    row_handles = {}
    local initial = {}
    for index = 1, count do initial[index] = index end
    local root = solid.mount(function()
        selected, set_selected = solid.create_signal(1)
        items, set_items = solid.create_signal(initial)
        local is_selected = solid.create_selector(selected)
        local list = solid.For({
            each = items,
            key = function(item) return item end,
            render = function(item)
                local label = gpuix.text("A representative nested row")
                local value = solid.text(function()
                    return is_selected(item()) and "selected" or ""
                end)
                local row = gpuix.div({ gpuix.text(item()), gpuix.div({ label, value }) })
                row_handles[item()] = row
                return row
            end,
        })
        return gpuix.div({ gpuix.div({ gpuix.text("Sidebar") }), gpuix.div({ list }) })
    end)
    __gpx_root(root)
end

function update(index)
    set_selected(index)
end

function reverse()
    local previous, next_items = items(), {}
    for index = #previous, 1, -1 do next_items[#next_items + 1] = previous[index] end
    set_items(next_items)
end

function verify(count)
    local original = row_handles[1]
    reverse()
    assert(items()[1] == count)
    assert(row_handles[1] == original, "For must preserve host identity")
    reverse()
    assert(items()[1] == 1)
end

function trim()
    set_items({ items()[1] })
end

function unmount()
    for index = #cleanups, 1, -1 do cleanups[index]() end
    cleanups = {}
    row_handles, selected, set_selected, items, set_items = nil, nil, nil, nil, nil
end
