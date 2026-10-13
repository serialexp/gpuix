import { createRequire } from "node:module"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"
import { describe, expect, it } from "vitest"

const { TestGpuixRenderer, hasTestGpuixRenderer } = createRequire(import.meta.url)(
  "../index.js"
)

describe.skipIf(!hasTestGpuixRenderer())("LuaX icons", () => {
  it("paints a bundled Phosphor icon from its token", () => {
    const screenshotDir = fs.mkdtempSync(path.join(os.tmpdir(), "gpuix-lua-icon-"))
    const shoot = (icon: string) => {
      const renderer = new TestGpuixRenderer(120, 120)
      renderer.loadLuax(`
        local Icon = require("gpuix.icon")
        return function()
          return <div style={{ width = 120, height = 120, background = "#000000" }}>
            ${icon}
          </div>
        end
      `)
      renderer.flush()
      const file = path.join(screenshotDir, `${icon ? "icon" : "empty"}.png`)
      renderer.captureScreenshot(file)
      return { renderer, file }
    }

    const { renderer, file: withIcon } = shoot(
      `<Icon testId="icon" name="Activity" weight="bold" size={96} color="#ffffff" />`
    )
    expect(renderer.getTreeJson()).toContain("phosphor:3:pulse")
    const { file: empty } = shoot("")
    if (!process.env.CI) {
      expect(fs.readFileSync(withIcon).equals(fs.readFileSync(empty))).toBe(false)
    }
  })

  it("rejects an unknown icon name when the component renders", () => {
    const renderer = new TestGpuixRenderer(120, 120)
    expect(() =>
      renderer.loadLuax(`
        local Icon = require("gpuix.icon")
        return function()
          return <Icon name="definitely-not-an-icon" />
        end
      `)
    ).toThrow(/Phosphor/)
  })
})
