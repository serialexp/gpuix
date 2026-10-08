local solid = require("gpuix.solid")
local count, set_count = solid.create_signal(0)

return function()
    return solid.mount(function()
        local label = solid.text(function() return "Count: " .. count() end)
        local button = gpuix.div {
            testId = "solid-counter",
            tabIndex = 0,
            autoFocus = true,
            children = { label },
            onClick = function()
                set_count(function(value) return value + 1 end)
            end,
        }
        solid.bind_style(button, function()
            return {
                width = 180,
                padding = 16,
                borderRadius = 8,
                color = "#ffffff",
                background = count() % 2 == 0 and "#2563eb" or "#7c3aed",
                focus = { borderWidth = 1, borderColor = "#ffffff" },
            }
        end)
        return gpuix.div {
            style = { padding = 24, background = "#18181b", height = "100%" },
            children = { button },
        }
    end)
end
