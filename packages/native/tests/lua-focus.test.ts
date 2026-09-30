import { createRequire } from "node:module"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"
import { describe, expect, it } from "vitest"

const { TestGpuixRenderer, hasTestGpuixRenderer } = createRequire(import.meta.url)(
  "../index.js"
)

describe.skipIf(!hasTestGpuixRenderer())("LuaX focus navigation", () => {
  it("activates focused buttons and tabs with Enter and Space", () => {
    const renderer = new TestGpuixRenderer(420, 180)
    renderer.loadLuax(`
      local Button = require("gpuix.button")

      return function()
        local presses, set_presses = gpuix.use_state(0)
        local tab, set_tab = gpuix.use_state("overview")
        return <div style={{ width = 420, height = 180 }}>
          <Button autoFocus testId="button" onClick={function()
            set_presses(function(value) return value + 1 end)
          end}><text>Run</text></Button>
          <div tabIndex={0} testId="tab" onClick={function()
            set_tab("widgets")
          end}><text>Widgets</text></div>
          <text>{"Presses: " .. presses .. "; tab: " .. tab}</text>
        </div>
      end
    `)
    renderer.flush()

    const activate = (key: string) => {
      renderer.simulateKeyDown(key)
      renderer.simulateKeyUp(key)
      for (const event of renderer.drainEvents()) renderer.dispatchLuaEvent(event)
      renderer.flush()
    }

    activate("enter")
    expect(renderer.getAllText()).toContain("Presses: 1; tab: overview")
    activate("space")
    expect(renderer.getAllText()).toContain("Presses: 2; tab: overview")
    renderer.simulateKeystrokes("tab")
    activate("enter")
    expect(renderer.getAllText()).toContain("Presses: 2; tab: widgets")
  })

  it("opens a focused Select once on Enter", () => {
    const renderer = new TestGpuixRenderer(420, 180)
    renderer.loadLuax(`
      local Select = require("gpuix.select")

      return function()
        return <Select
          testId="runtime"
          defaultValue="lua54"
          options={{
            { value = "lua54", label = "Lua 5.4" },
            { value = "luajit", label = "LuaJIT" },
          }}
        />
      end
    `)
    renderer.flush()

    const tree = JSON.parse(renderer.getTreeJson()) as {
      id: number
      testId?: string
      children?: unknown[]
    }
    const findId = (node: typeof tree): number | undefined => {
      if (node.testId === "runtime") return node.id
      for (const child of node.children ?? []) {
        const found = findId(child as typeof tree)
        if (found !== undefined) return found
      }
      return undefined
    }
    const selectId = findId(tree)
    expect(selectId).toBeDefined()
    renderer.focusElement(selectId!)

    renderer.simulateKeyDown("enter")
    renderer.simulateKeyUp("enter")
    for (const event of renderer.drainEvents()) renderer.dispatchLuaEvent(event)
    renderer.flush()

    expect(renderer.getTreeJson()).toContain("runtime-content")
  })

  it("cycles forward and backward through native tab stops", () => {
    const renderer = new TestGpuixRenderer(420, 180)
    const screenshotDir = fs.mkdtempSync(path.join(os.tmpdir(), "gpuix-lua-focus-"))
    const beforeFocus = path.join(screenshotDir, "before.png")
    const afterFocus = path.join(screenshotDir, "after.png")
    renderer.loadLuax(`
      local Button = require("gpuix.button")
      local button_style = {
        width = 120,
        height = 40,
        background = "#18181b",
        focusVisible = { background = "#ff00ff" },
      }

      return function()
        local selected, set_selected = gpuix.use_state("none")
        return <div style={{ width = 420, height = 180 }}>
          <Button autoFocus testId="first" style={button_style} onKeyDown={function(event)
            if event.key == "a" then set_selected("first") end
          end}>
            <text>First</text>
          </Button>
          <Button testId="second" style={button_style} onKeyDown={function(event)
            if event.key == "a" then set_selected("second") end
          end}>
            <text>Second</text>
          </Button>
          <Button tabIndex={-1} testId="skipped" style={button_style} onKeyDown={function(event)
            if event.key == "a" then set_selected("skipped") end
          end}>
            <text>Skipped</text>
          </Button>
          <text>{"Selected: " .. selected}</text>
        </div>
      end
    `)
    renderer.flush()
    expect(renderer.getTreeJson()).toContain('"focusVisible"')
    renderer.captureScreenshot(beforeFocus)

    const dispatchEvents = () => {
      for (const event of renderer.drainEvents()) renderer.dispatchLuaEvent(event)
      renderer.flush()
    }

    renderer.simulateKeystrokes("tab")
    renderer.flush()
    renderer.captureScreenshot(afterFocus)
    if (!process.env.CI) {
      expect(fs.readFileSync(afterFocus).equals(fs.readFileSync(beforeFocus))).toBe(false)
    }
    renderer.simulateKeystrokes("a")
    dispatchEvents()
    expect(renderer.getAllText()).toContain("Selected: second")

    renderer.simulateKeystrokes("tab a")
    dispatchEvents()
    expect(renderer.getAllText()).toContain("Selected: first")

    renderer.simulateKeystrokes("shift-tab a")
    dispatchEvents()
    expect(renderer.getAllText()).toContain("Selected: second")
    expect(renderer.getAllText()).not.toContain("Selected: skipped")
    fs.rmSync(screenshotDir, { recursive: true, force: true })
  })

  it("moves radio focus with selection without corrupting tab order", () => {
    const renderer = new TestGpuixRenderer(420, 180)
    renderer.loadLuax(`
      local Button = require("gpuix.button")
      local RadioGroup = require("gpuix.radio_group")

      return function()
        local value, set_value = gpuix.use_state("comfortable")
        local after, set_after = gpuix.use_state("idle")
        return <div style={{ width = 420, height = 180 }}>
          <RadioGroup
            testId="density"
            value={value}
            onValueChange={set_value}
            options={{
              { value = "comfortable", label = "Comfort" },
              { value = "compact", label = "Compact" },
            }}
          />
          <Button testId="after" onKeyDown={function(event)
            if event.key == "a" then set_after("focused") end
          end}><text>After</text></Button>
          <text>{"Selected: " .. value .. "; after: " .. after}</text>
        </div>
      end
    `)
    renderer.flush()

    const tree = JSON.parse(renderer.getTreeJson()) as {
      id: number
      testId?: string
      children?: unknown[]
    }
    const findId = (node: typeof tree, testId: string): number | undefined => {
      if (node.testId === testId) return node.id
      for (const child of node.children ?? []) {
        const found = findId(child as typeof tree, testId)
        if (found !== undefined) return found
      }
      return undefined
    }
    const initialRadioId = findId(tree, "density-comfortable")
    expect(initialRadioId).toBeDefined()
    renderer.focusElement(initialRadioId!)

    const press = (key: string) => {
      renderer.simulateKeystrokes(key)
      for (const event of renderer.drainEvents()) renderer.dispatchLuaEvent(event)
      renderer.flush()
    }

    press("right")
    expect(renderer.getAllText()).toContain("Selected: compact; after: idle")
    press("left")
    expect(renderer.getAllText()).toContain("Selected: comfortable; after: idle")

    press("right")
    press("tab")
    press("a")
    expect(renderer.getAllText()).toContain("Selected: compact; after: focused")

    press("shift-tab")
    press("left")
    expect(renderer.getAllText()).toContain("Selected: comfortable; after: focused")
  })

  it("focuses a control through its children and blurs on background press", () => {
    const renderer = new TestGpuixRenderer(420, 220)
    renderer.loadLuax(`
      local Button = require("gpuix.button")
      local Checkbox = require("gpuix.checkbox")

      return function()
        local target, set_target = gpuix.use_state("none")
        return <div testId="root" style={{ width = 420, height = 220, display = "flex", flexDirection = "column", gap = 12 }}>
          <Button autoFocus testId="first" style={{ width = 120, height = 40 }} onKeyDown={function(event)
            if event.key == "f" then set_target("first") end
          end}><text>First</text></Button>
          <Checkbox testId="checkbox" defaultChecked label="Enabled" />
          <Button testId="after" style={{ width = 120, height = 40 }} onKeyDown={function(event)
            if event.key == "a" then set_target("after") end
          end}><text>After</text></Button>
          <text>{"Target: " .. target}</text>
        </div>
      end
    `)
    renderer.flush()
    expect(renderer.getTreeJson()).toContain('"focus"')

    const tree = JSON.parse(renderer.getTreeJson()) as {
      id: number
      testId?: string
      children?: unknown[]
    }
    const findId = (node: typeof tree, testId: string): number | undefined => {
      if (node.testId === testId) return node.id
      for (const child of node.children ?? []) {
        const found = findId(child as typeof tree, testId)
        if (found !== undefined) return found
      }
      return undefined
    }
    const click = (testId: string) => {
      const id = findId(tree, testId)
      expect(id).toBeDefined()
      const bounds = renderer.getElementBounds(id!)
      expect(bounds).not.toBeNull()
      const [x, y, width, height] = bounds!
      renderer.simulateClick(x + width / 2, y + height / 2)
      for (const event of renderer.drainEvents()) renderer.dispatchLuaEvent(event)
      renderer.flush()
    }
    const press = (keys: string) => {
      renderer.simulateKeystrokes(keys)
      for (const event of renderer.drainEvents()) renderer.dispatchLuaEvent(event)
      renderer.flush()
    }

    click("checkbox-indicator")
    press("tab a")
    expect(renderer.getAllText()).toContain("Target: after")

    click("checkbox-indicator")
    const rootBounds = renderer.getElementBounds(tree.id)
    expect(rootBounds).not.toBeNull()
    const [rootX, rootY, rootWidth, rootHeight] = rootBounds!
    renderer.simulateClick(rootX + rootWidth - 5, rootY + rootHeight - 5)
    renderer.flush()
    press("tab f")
    expect(renderer.getAllText()).toContain("Target: first")
  })

  it("attaches native focus handles to custom leaves", () => {
    const renderer = new TestGpuixRenderer(420, 180)
    renderer.loadLuax(`
      local Button = require("gpuix.button")
      return function()
        local target, set_target = gpuix.use_state("none")
        return <div style={{ width = 420, height = 180, display = "flex", flexDirection = "column" }}>
          <Button autoFocus><text>First</text></Button>
          <svg
            testId="icon"
            tabIndex={0}
            source="<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'><path d='M0 0h10v10H0z'/></svg>"
            style={{ width = 24, height = 24 }}
          />
          <Button testId="after" onKeyDown={function(event)
            if event.key == "a" then set_target("after") end
          end}><text>After</text></Button>
          <text>{"Target: " .. target}</text>
        </div>
      end
    `)
    renderer.flush()

    const tree = JSON.parse(renderer.getTreeJson()) as {
      id: number
      testId?: string
      children?: unknown[]
    }
    const icon = (tree.children ?? []).find(
      (child) => (child as { testId?: string }).testId === "icon"
    ) as { id: number }
    const [x, y, width, height] = renderer.getElementBounds(icon.id)!
    renderer.simulateClick(x + width / 2, y + height / 2)
    renderer.simulateKeystrokes("tab a")
    for (const event of renderer.drainEvents()) renderer.dispatchLuaEvent(event)
    renderer.flush()
    expect(renderer.getAllText()).toContain("Target: after")
  })
})
