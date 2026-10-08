local host = require("gpuix.host")
local api = {}
local current_computation
local current_owner
local batch_depth = 0
local queue = {}
local memo_queue = {}
local queued = {}
local flushing = false
local run

local function schedule(computation)
    if computation.disposed then return end
    if computation.memo and not computation.dirty then
        computation.dirty = true
        for observer in pairs(computation.output.observers) do
            if observer.memo then schedule(observer) end
        end
    end
    if not queued[computation] then
        queued[computation] = true
        local pending = computation.memo and memo_queue or queue
        pending[#pending + 1] = computation
    end
end

local function cleanup(owner)
    for index = #owner.children, 1, -1 do
        api.dispose(owner.children[index])
    end
    owner.children = {}
    for index = #owner.cleanups, 1, -1 do owner.cleanups[index]() end
    owner.cleanups = {}
    if owner.sources then
        for source in pairs(owner.sources) do source.observers[owner] = nil end
        owner.sources = {}
    end
end

function api.dispose(owner)
    if owner.disposed then return end
    owner.disposed = true
    queued[owner] = nil
    cleanup(owner)
    if owner.parent then
        for index, child in ipairs(owner.parent.children) do
            if child == owner then
                table.remove(owner.parent.children, index)
                break
            end
        end
        owner.parent = nil
    end
end

run = function(computation)
    if computation.disposed then return end
    assert(not computation.running, "Circular memo dependency")
    computation.running = true
    computation.dirty = false
    queued[computation] = nil
    cleanup(computation)
    local previous_computation, previous_owner = current_computation, current_owner
    current_computation, current_owner = computation, computation
    local ok, result = pcall(computation.callback)
    current_computation, current_owner = previous_computation, previous_owner
    computation.running = false
    if not ok then error(result, 0) end
end

local function flush()
    if flushing or batch_depth > 0 then return end
    flushing = true
    local ok, failure = pcall(function()
        local memo_index, effect_index = 1, 1
        while memo_index <= #memo_queue or effect_index <= #queue do
            local computation
            if memo_index <= #memo_queue then
                computation = memo_queue[memo_index]
                memo_index = memo_index + 1
            else
                computation = queue[effect_index]
                effect_index = effect_index + 1
            end
            if queued[computation] then run(computation) end
        end
    end)
    queue, memo_queue, queued = {}, {}, {}
    flushing = false
    if not ok then error(failure, 0) end
end

local function new_owner()
    local owner = { children = {}, cleanups = {}, parent = current_owner }
    if current_owner then
        current_owner.children[#current_owner.children + 1] = owner
    end
    return owner
end

local function create_signal(initial, producer)
    local source = { value = initial, observers = {}, producer = producer }
    if producer then producer.output = source end
    local function read()
        assert(not producer or not producer.running, "Circular memo dependency")
        if producer and producer.dirty and not producer.disposed then run(producer) end
        if current_computation then
            source.observers[current_computation] = true
            current_computation.sources[source] = true
        end
        return source.value
    end
    local function write(next_value)
        if type(next_value) == "function" then next_value = next_value(source.value) end
        if next_value == source.value then return source.value end
        source.value = next_value
        for computation in pairs(source.observers) do
            schedule(computation)
        end
        flush()
        return source.value
    end
    return read, write
end

function api.create_signal(initial)
    return create_signal(initial)
end

function api.create_memo(callback)
    assert(current_owner, "create_memo requires a create_root owner")
    assert(type(callback) == "function", "create_memo requires a function")
    local computation = new_owner()
    computation.memo = true
    computation.sources = {}
    local read, write = create_signal(nil, computation)
    computation.callback = function()
        local value = callback()
        write(function() return value end)
    end
    local ok, failure = pcall(run, computation)
    if not ok then
        api.dispose(computation)
        error(failure, 0)
    end
    return read
end

function api.create_effect(callback)
    assert(current_owner, "create_effect requires a create_root owner")
    local computation = new_owner()
    computation.sources = {}
    computation.callback = callback
    local ok, failure = pcall(run, computation)
    if not ok then
        api.dispose(computation)
        error(failure, 0)
    end
    return computation
end

function api.create_root(callback)
    local owner = new_owner()
    local previous_owner, previous_computation = current_owner, current_computation
    current_owner, current_computation = owner, nil
    local ok, result = pcall(callback, owner)
    current_owner, current_computation = previous_owner, previous_computation
    if not ok then
        api.dispose(owner)
        error(result, 0)
    end
    return result, owner
end

function api.on_cleanup(callback)
    assert(current_owner, "on_cleanup requires an owner")
    current_owner.cleanups[#current_owner.cleanups + 1] = callback
end

function api.untrack(callback)
    local previous = current_computation
    current_computation = nil
    local ok, result = pcall(callback)
    current_computation = previous
    if not ok then error(result, 0) end
    return result
end

function api.batch(callback)
    batch_depth = batch_depth + 1
    local ok, result = pcall(callback)
    batch_depth = batch_depth - 1
    flush()
    if not ok then error(result, 0) end
    return result
end

function api.text(read)
    local handle = host.text(api.untrack(read))
    api.create_effect(function() host.set_text(handle, read()) end)
    return handle
end

function api.mount(component)
    local result, owner = api.create_root(component)
    host.on_root_cleanup(function() api.dispose(owner) end)
    return result
end

function api.use_store(store, selector, equality)
    assert(current_owner, "use_store requires an owner")
    assert(type(store) == "table" and type(store.get_state) == "function"
        and type(store.subscribe) == "function", "use_store requires a subscribable store")
    assert(selector == nil or type(selector) == "function", "use_store selector must be a function")
    assert(equality == nil or type(equality) == "function", "use_store equality must be a function")
    local function select()
        local state = store.get_state()
        if selector then return selector(state) end
        return state
    end
    local selected = api.untrack(select)
    local read, write = api.create_signal(selected)
    local active = true
    local unsubscribe = store.subscribe(function()
        if not active then return end
        api.untrack(function()
            local next_value = select()
            local equal
            if equality then equal = equality(selected, next_value)
            else equal = selected == next_value end
            if not equal then
                selected = next_value
                write(function() return next_value end)
            end
        end)
    end)
    api.on_cleanup(function()
        active = false
        unsubscribe()
    end)
    return read
end

function api.bind_style(handle, read)
    return api.create_effect(function() host.set_style(handle, read()) end)
end

function api.h(kind, props)
    if type(kind) == "function" then return kind(props) end
    local style = type(props.style) == "function" and props.style or nil
    local content = kind == "text" and type(props.content) == "function" and props.content or nil
    if style then props.style = api.untrack(style) end
    if content then props.content = api.untrack(content) end
    local bindings = {}
    local excluded = { style = true, content = true, key = true, children = true,
        autoFocus = true, testId = true, ref = true, className = true }
    for name, value in pairs(props) do
        if type(name) == "string" and not excluded[name]
            and string.sub(name, 1, 2) ~= "on" and type(value) == "function" then
            bindings[name] = value
            props[name] = api.untrack(value)
        end
    end
    local handle = gpuix.h(kind, props)
    if style then api.bind_style(handle, style) end
    for name, read in pairs(bindings) do api.bind_prop(handle, name, read) end
    if content then
        api.create_effect(function() host.set_text(handle, content()) end)
    end
    return handle
end

function api.bind_prop(handle, name, read)
    return api.create_effect(function() host.set_prop(handle, name, read()) end)
end

function api.Show(props)
    assert(current_owner, "Show requires a mounted owner")
    assert(type(props.when) == "function", "Show.when must be a signal accessor")
    assert(type(props.render) == "function", "Show.render must be a lazy function")
    local parent = current_owner
    local slot = gpuix.div { testId = props.testId, style = props.style }
    local branch_owner
    local previous
    api.on_cleanup(function()
        if branch_owner then api.dispose(branch_owner) end
    end)
    api.create_effect(function()
        local visible = not not props.when()
        if previous == visible then return end
        previous = visible
        if branch_owner then api.dispose(branch_owner) end
        branch_owner = nil
        local render = visible and props.render or props.fallback
        local child
        if render then
            local saved_owner = current_owner
            current_owner = parent
            local ok, result, owner = pcall(api.create_root, render)
            current_owner = saved_owner
            if not ok then error(result, 0) end
            child, branch_owner = result, owner
        end
        host.set_child(slot, child)
    end)
    return slot
end

local function create_list(props, indexed)
    local name = indexed and "Index" or "For"
    assert(current_owner, name .. " requires a mounted owner")
    assert(type(props.each) == "function", name .. ".each must be an accessor")
    if not indexed then
        assert(type(props.key) == "function", "For.key must be a function")
    end
    assert(type(props.render) == "function", name .. ".render must be a lazy function")
    local parent = current_owner
    local slot = gpuix.div { testId = props.testId, style = props.style }
    local rows = {}
    api.on_cleanup(function()
        for _, row in pairs(rows) do api.dispose(row.owner) end
    end)
    api.create_effect(function()
        local items = props.each()
        assert(type(items) == "table", name .. ".each must return a dense array")
        api.untrack(function()
            local keys, seen, count = {}, {}, 0
            for index in pairs(items) do
                assert(type(index) == "number" and index >= 1 and index % 1 == 0,
                    name .. ".each must return a dense array")
                count = count + 1
            end
            for index = 1, count do
                assert(items[index] ~= nil, name .. ".each must return a dense array")
                local key
                if indexed then key = index else key = props.key(items[index]) end
                assert((type(key) == "string" or type(key) == "number") and key == key,
                    "For keys must be strings or numbers")
                assert(not seen[key], "For keys must be unique")
                keys[index], seen[key] = key, true
            end
            api.batch(function()
                local children = {}
                for index, key in ipairs(keys) do
                    local row = rows[key]
                    if not row then
                        local item, set_item = api.create_signal(items[index])
                        local position, set_position
                        if indexed then position = index
                        else position, set_position = api.create_signal(index) end
                        local saved_owner = current_owner
                        current_owner = parent
                        local ok, handle, owner = pcall(api.create_root, function()
                            local child = props.render(item, position)
                            assert(type(child) == "number", name .. ".render must return a host handle")
                            return child
                        end)
                        current_owner = saved_owner
                        if not ok then error(handle, 0) end
                        row = { handle = handle, owner = owner,
                            set_item = set_item, set_position = set_position }
                        rows[key] = row
                    else
                        row.set_item(items[index])
                        if row.set_position then row.set_position(index) end
                    end
                    children[index] = row.handle
                end
                host.set_children(slot, children)
                for key, row in pairs(rows) do
                    if not seen[key] then
                        api.dispose(row.owner)
                        rows[key] = nil
                    end
                end
            end)
        end)
    end)
    return slot
end

function api.For(props)
    return create_list(props, false)
end

function api.Index(props)
    return create_list(props, true)
end

return api
