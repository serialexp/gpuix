import { createRequire } from "node:module"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"
import { describe, expect, it } from "vitest"

const { TestGpuixRenderer } = createRequire(import.meta.url)("../index.js")

describe("LuaX focus navigation", () => {
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
})
