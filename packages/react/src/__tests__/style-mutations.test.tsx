import { afterEach, describe, expect, it } from "vitest"
import type { NativeRenderer } from "../types/host.js"
import { createRoot, flushSync, type Root } from "../reconciler/reconciler.js"

class RecordingRenderer implements NativeRenderer {
  batches: unknown[][] = []
  snapshots: unknown[] = []

  applyBatch(json: string): number[] {
    this.batches.push(...JSON.parse(json))
    return []
  }

  applySnapshot(json: string): number[] {
    this.snapshots.push(JSON.parse(json))
    return []
  }
}

describe("style mutation transport", () => {
  let root: Root | null = null

  afterEach(() => {
    root?.unmount()
    root = null
  })

  it("skips structurally unchanged styles and sends changed styles", () => {
    const renderer = new RecordingRenderer()
    root = createRoot(renderer)
    const style = { color: "red", padding: 8 }

    flushSync(() => root!.render(<div style={style}>first</div>))
    renderer.batches = []

    flushSync(() =>
      root!.render(<div style={{ color: "red", padding: 8 }}>second</div>)
    )
    expect(renderer.batches.filter(([name]) => name === "setStyle")).toEqual([])

    renderer.batches = []
    flushSync(() =>
      root!.render(<div style={{ color: "red", padding: 12 }}>third</div>)
    )
    expect(renderer.batches.filter(([name]) => name === "setStyle")).toEqual([
      ["setStyle", 2, { color: "red", padding: 12 }],
    ])
  })

  it("can send an authoritative host snapshot instead of mutations", () => {
    const renderer = new RecordingRenderer()
    root = createRoot(renderer, { transport: "snapshot" })

    flushSync(() =>
      root!.render(
        <div style={{ color: "red" }} onClick={() => {}}>
          first
        </div>
      )
    )
    expect(renderer.batches).toEqual([])
    expect(renderer.snapshots).toHaveLength(1)
    expect(renderer.snapshots[0]).toMatchObject({
      rootId: 2,
      nodes: [
        [2, "div", { color: "red" }, null, ["click"], [1], {}],
        [1, "text", null, "first", [], [], {}],
      ],
    })

    flushSync(() =>
      root!.render(
        <div style={{ color: "red" }} onClick={() => {}}>
          second
        </div>
      )
    )
    expect(renderer.snapshots).toHaveLength(2)
    expect(renderer.snapshots[1]).toMatchObject({
      rootId: 2,
      nodes: [
        [2, "div", { color: "red" }, null, ["click"], [1], {}],
        [1, "text", null, "second", [], [], {}],
      ],
    })
  })
})
