import { readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"
import { GpuixRenderer, type EventPayload } from "../packages/native/index.js"

let renderer: GpuixRenderer
renderer = new GpuixRenderer((error: Error | null, event?: EventPayload) => {
  if (error) {
    console.error("[GPUIX Lua]", error)
  } else if (event) {
    renderer.dispatchLuaEvent(event)
  }
})

const source = readFileSync(
  fileURLToPath(new URL("./lua-counter.lua", import.meta.url)),
  "utf8"
)
renderer.loadLua(source)
renderer.init({ title: "GPUIX Lua Counter", width: 640, height: 420 })

if (process.platform === "darwin") {
  const tick = () => {
    if (renderer.tick()) setTimeout(tick, 8)
  }
  tick()
}
