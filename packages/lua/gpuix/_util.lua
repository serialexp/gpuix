local util = {}

util.focus_ring = {
    boxShadow = {
        offsetX = 0,
        offsetY = 0,
        blurRadius = 0,
        spreadRadius = 2,
        color = "#60a5fa",
    },
}

function util.resolve_style(style, state, fallback)
    if type(style) == "function" then return style(state) end
    if style ~= nil then return style end
    return fallback
end

function util.test_id(base, suffix)
    if base == nil then return nil end
    return base .. suffix
end

function util.call(callback, value)
    if callback ~= nil then callback(value) end
end

function util.option_index(options, value)
    for index, option in ipairs(options) do
        if option.value == value then return index end
    end
    return nil
end

function util.is_disabled(option)
    return option.disabled == true
end

return util
