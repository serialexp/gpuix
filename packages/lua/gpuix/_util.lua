local util = {}
util.palette = gpuix.theme()

util.focus_ring = {
    foregroundBoxShadow = {
        offsetX = 0,
        offsetY = 0,
        blurRadius = 0,
        spreadRadius = util.palette.metrics.controlRingWidth,
        color = util.palette.focusRing,
        inset = true,
    },
}

function util.focus_style(palette)
    return { borderColor = palette.accent, foregroundBoxShadow = {
        offsetX = 0, offsetY = 0, blurRadius = 0,
        spreadRadius = palette.metrics.controlRingWidth,
        color = palette.focusRing, inset = true,
    } }
end

function util.control_styles(palette)
    local metrics = palette.metrics
    local focus = util.focus_style(palette)
    return {
        row = { display = "flex", flexDirection = "row", alignItems = "center",
            gap = metrics.controlGap, color = palette.text, focus = focus },
        button = { display = "flex", flexDirection = "row", alignItems = "center", justifyContent = "center",
            gap = metrics.controlGap, height = metrics.inputHeight, paddingLeft = metrics.controlPadding,
            paddingRight = metrics.controlPadding, borderRadius = metrics.inputRadius,
            background = palette.raised, color = palette.text, focus = focus,
            hover = { background = palette.buttonHover }, active = { background = palette.buttonPressed } },
        trigger = { width = 200, height = metrics.inputHeight, display = "flex", flexDirection = "row",
            alignItems = "center", gap = metrics.controlGap, paddingLeft = metrics.controlPadding,
            paddingRight = metrics.controlPadding, background = palette.field, color = palette.text,
            borderWidth = 1, borderColor = palette.borderStrong, borderRadius = metrics.inputRadius,
            hover = { borderColor = palette.scrollbarThumbHover }, focus = focus },
        menu = { width = 200, display = "flex", flexDirection = "column", padding = metrics.controlPadding / 2,
            borderWidth = 1, borderColor = palette.borderStrong, borderRadius = metrics.menuRadius,
            boxShadow = { offsetX = 0, offsetY = metrics.controlPadding / 2, blurRadius = metrics.controlPadding * 2,
                spreadRadius = 0, color = palette.shadow },
            background = palette.raised, color = palette.text, focus = focus },
        item = { width = "100%", height = metrics.menuItemHeight, display = "flex", flexDirection = "row",
            alignItems = "center", gap = metrics.controlGap, paddingLeft = metrics.controlPadding,
            paddingRight = metrics.controlPadding, borderRadius = metrics.inputRadius,
            color = palette.text, hover = { background = palette.rowHover }, focus = focus },
    }
end

function util.indicator_style(palette, state, radio)
    local size = palette.metrics.controlIndicatorSize
    return { width = size, height = size, display = "flex", alignItems = "center", justifyContent = "center",
        flexShrink = 0, borderRadius = radio and size / 2 or palette.metrics.inputRadius / 2,
        borderWidth = 1, borderColor = (state.checked or state.selected) and palette.accent or palette.borderStrong,
        background = state.checked and palette.accent or palette.field,
        color = state.checked and palette.onAccent or palette.text,
        pointerEvents = "auto" }
end

function util.menu_item_style(palette, state)
    local style = util.control_styles(palette).item
    style.background = state.highlighted and palette.rowHover or "transparent"
    style.color = state.disabled and palette.textFaint or palette.text
    return style
end

function util.row_style(palette, state)
    local style = util.control_styles(palette).row
    style.opacity = state.disabled and 0.45 or 1
    return style
end

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
