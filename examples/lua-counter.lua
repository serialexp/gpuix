local ui = gpuix
local root_style = ui.style {
    width = "100%",
    height = "100%",
    display = "flex",
    flexDirection = "column",
    alignItems = "center",
    justifyContent = "center",
    gap = 16,
    background = "#111827",
}
local count_style = ui.style { color = "#f9fafb", fontSize = 28 }
local button_style = ui.style {
    padding = 12,
    borderRadius = 8,
    background = "#2563eb",
    color = "#ffffff",
}

local function Counter()
    local count, set_count = ui.use_state(0)

    return ui.div {
        style = root_style,
        ui.text {
            style = count_style,
            content = "Count: " .. count,
        },
        ui.div {
            key = "increment",
            testId = "increment",
            style = button_style,
            onClick = function()
                set_count(function(value) return value + 1 end)
            end,
            ui.text("Increment"),
        },
    }
end

return Counter
