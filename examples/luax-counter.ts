import { readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"
import { GpuixRenderer, type EventPayload } from "../packages/native/index.js"

let renderer: GpuixRenderer
renderer = new GpuixRenderer((error: Error | null, event?: EventPayload) => {
  if (error) {
    console.error("[GPUIX LuaX]", error)
  } else if (event) {
    renderer.dispatchLuaEvent(event)
  }
})

const source = readFileSync(
  fileURLToPath(new URL("./luax-counter.luax", import.meta.url)),
  "utf8"
)
renderer.loadLuax(source)
renderer.init({ title: "GPUIX LuaX Counter", width: 640, height: 420 })

if (process.platform === "darwin") {
  const tick = () => {
    if (renderer.tick()) setTimeout(tick, 8)
  }
  tick()
}
