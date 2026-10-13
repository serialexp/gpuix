local solid = require("gpuix.solid")
local util = require("gpuix._util")
local shared = {}

function shared.read(props, name, fallback)
    local value = props[name]
    if type(value) == "function" then value = value() end
    if value == nil then return fallback end
    return value
end

function shared.controlled(props, name, initial, callback)
    local local_value, set_local = solid.create_signal(initial)
    local function value()
        if props[name] ~= nil then return shared.read(props, name) end
        return local_value()
    end
    local function set_value(next_value)
        if value() == next_value then return end
        if props[name] == nil then set_local(function() return next_value end) end
        util.call(props[callback], next_value)
    end
    return value, set_value
end

function shared.merge(base, override)
    local result = {}
    for key, value in pairs(base) do result[key] = value end
    if override then
        for key, value in pairs(override) do result[key] = value end
    end
    return result
end

function shared.style(base, override, state)
    if type(override) == "function" then override = override(state) end
    if type(override) == "number" or type(override) == "userdata" then return override end
    return shared.merge(base, override)
end

function shared.contains(values, value)
    if type(values) ~= "table" then return false end
    for _, candidate in ipairs(values) do
        if candidate == value then return true end
    end
    return false
end

return shared
