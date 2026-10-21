local ui = gpuix
local app = ui.create_app()
local tray
local paused = false

local function tray_options()
    return {
        icon = "icon.svg",
        tooltip = paused and "GPUIX Tray (paused)" or "GPUIX Tray",
        menu = {
            { label = "Open workspace", onClick = function() ui.open_window(app, "main") end },
            { label = "Paused", checked = paused, onClick = function()
                paused = not paused
                ui.update_tray(tray, tray_options())
            end },
            { label = "Managed by Lua", enabled = false, onClick = function() end },
            { separator = true },
            { label = "Quit", onClick = function() ui.quit(app) end },
        },
    }
end

local function Main()
    return ui.div {
        style = {
            width = "100%", height = "100%", padding = 20,
            display = "flex", flexDirection = "column", gap = 16,
            background = "#111827", color = "#f9fafb",
        },
        ui.div {
            testId = "hide-window",
            style = { height = 40, padding = 8, background = "#6366f1", borderRadius = 6 },
            onClick = function() ui.close_window(app, "main") end,
            ui.text("Close window"),
        },
        ui.text("The app stays in the menu bar. Choose Open to return, or Quit to exit."),
    }
end

ui.define_window(app, {
    id = "main", title = "GPUIX Tray", width = 420, height = 200, render = Main,
})
tray = ui.create_tray(app, tray_options())
return app
