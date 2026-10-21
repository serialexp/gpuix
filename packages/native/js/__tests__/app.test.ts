import { afterEach, describe, expect, it } from "vitest"
import { createTray, updateTray, removeTray, showWindow, hideWindow, quit } from "../app.js"

const globals = globalThis as unknown as { __gpuix?: unknown }
const original = globals.__gpuix
afterEach(() => { globals.__gpuix = original })

function bridge() {
  let next = 1
  let dispatch: (action: number) => void = () => {}
  const commands: any[] = []
  globals.__gpuix = {
    allocateTrayId: () => next++,
    trayCommand(json: string) {
      const command = JSON.parse(json)
      if (command.spec?.icon === "invalid") throw new Error("invalid icon")
      commands.push(command)
    },
    setTrayHandler(handler: typeof dispatch) { dispatch = handler },
  }
  return { commands, click: (action: number) => dispatch(action) }
}

describe("application-owned trays", () => {
  it("updates callbacks and state without serializing functions", () => {
    const app = bridge()
    const calls: string[] = []
    const tray = createTray({ icon: "icon.svg", menu: [{ label: "First", onClick: () => calls.push("first") }] })
    const first = app.commands[0].spec.menu[0].action
    app.click(first)
    updateTray(tray, { icon: "icon.svg", tooltip: "Updated", menu: [
      { label: "Second", checked: true, enabled: false, onClick: () => calls.push("second") },
      { separator: true },
    ] })
    const second = app.commands[1].spec.menu[0].action
    expect(app.commands[1].spec.menu).toEqual([{ label: "Second", action: second, checked: true, enabled: false }, { separator: true }])
    app.click(first)
    app.click(second)
    expect(calls).toEqual(["first", "second"])
    removeTray(tray)
    app.click(second)
    removeTray(tray)
    expect(calls).toEqual(["first", "second"])
    expect(app.commands.filter(command => command.method === "remove")).toHaveLength(1)
    expect(() => updateTray(tray, { icon: "icon.svg", menu: [] })).toThrow("removed")
  })

  it("keeps previous callbacks after a rejected update", () => {
    const app = bridge()
    let clicks = 0
    const tray = createTray({ icon: "icon.svg", menu: [{ label: "Click", onClick: () => clicks++ }] })
    expect(() => updateTray(tray, { icon: "invalid", menu: [] })).toThrow("invalid icon")
    app.click(app.commands[0].spec.menu[0].action)
    expect(clicks).toBe(1)
    removeTray(tray)
  })

  it("keeps separate trays and queues application commands", () => {
    const app = bridge()
    const first = createTray({ icon: "icon.svg", menu: [] })
    const second = createTray({ icon: "other.svg", menu: [] })
    expect(first.id).not.toBe(second.id)
    removeTray(first)
    updateTray(second, { icon: "other.svg", menu: [] })
    showWindow(); hideWindow(); quit()
    expect(app.commands.slice(-3)).toEqual([{ method: "showWindow" }, { method: "hideWindow" }, { method: "quit" }])
    removeTray(second)
  })

  it("imports without loading native code and rejects unsupported runtimes", () => {
    globals.__gpuix = undefined
    expect(() => createTray({ icon: "icon.svg", menu: [] })).toThrow("gpx")
  })
})
