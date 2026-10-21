import React from "react"
import { render } from "@gpuix/react"
import { createTray, updateTray, showWindow, hideWindow, quit } from "@gpuix/native/app"
import type { TrayOptions } from "@gpuix/native/app"

let paused = false

function options(): TrayOptions {
  return {
    icon: "icon.svg",
    tooltip: paused ? "GPUIX JavaScript Tray (paused)" : "GPUIX JavaScript Tray",
    menu: [
      { label: "Open workspace", onClick: showWindow },
      { label: "Paused", checked: paused, onClick() {
        paused = !paused
        updateTray(tray, options())
      } },
      { label: "Managed by JavaScript", enabled: false, onClick() {} },
      { separator: true },
      { label: "Quit", onClick: quit },
    ],
  }
}

const tray = createTray(options())

function App() {
  const [count, setCount] = React.useState(0)
  return <div style={{ display: "flex", width: "100%", height: "100%", padding: 20, gap: 12, flexDirection: "column", background: "#111827", color: "#f9fafb" }}>
    <div testId="increment" onClick={() => setCount(count + 1)} style={{ padding: 8, background: "#6366f1" }}>
      <text>{`Count: ${count}`}</text>
    </div>
    <div testId="hide-window" onClick={hideWindow} style={{ padding: 8, background: "#374151" }}>
      <text>Close window; reopen from the menu bar</text>
    </div>
  </div>
}

render(<App />, { title: "GPUIX JavaScript Tray", width: 420, height: 220 })
