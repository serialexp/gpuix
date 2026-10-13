import { useState } from "react"
import { describe, expect, it } from "vitest"
import { createRendererState } from "@gpuix/native/host"
import { createRoot, flushSync } from "../reconciler/reconciler.js"

type Op = [string, ...unknown[]]

/** A renderer that only records the batches it is sent. */
function recordingRenderer() {
  const batches: Op[][] = []
  return {
    batches,
    applyBatch(json: string): number[] {
      batches.push(JSON.parse(json) as Op[])
      return []
    },
  }
}

/** Mount a counter that counts `eventType` events, and send it one. */
function countOnce(eventType: "click" | "mouseMove") {
  const renderer = recordingRenderer()
  const root = createRoot(renderer)
  function Counter() {
    const [count, setCount] = useState(0)
    const increment = () => setCount((value) => value + 1)
    return (
      <div {...(eventType === "click" ? { onClick: increment } : { onMouseMove: increment })}>
        <text>{`Count: ${count}`}</text>
      </div>
    )
  }
  flushSync(() => root.render(<Counter />))
  const div = renderer.batches.flat().find((op) => op[0] === "createElement" && op[2] === "div")
  expect(div).toBeDefined()
  const before = renderer.batches.length
  createRendererState(renderer).dispatch({ elementId: div![1] as number, eventType })
  expect(renderer.batches.length).toBe(before)
  return {
    committed: () => renderer.batches.slice(before).flat().some(
      (op) => op[0] === "setText" && op[2] === "Count: 1"
    ),
    unmount: () => flushSync(() => root.unmount()),
  }
}

// React queued its microtask first, so it has run once this one does.
const afterMicrotasks = () => new Promise<void>((resolve) => queueMicrotask(resolve))
const afterTimer = () => new Promise<void>((resolve) => setTimeout(resolve, 0))

describe("update scheduling", () => {
  // As in React DOM. An embedded host runs the microtasks an event queued
  // before it returns, so it sees the commit without running its timers.
  it("commits a click's update in a microtask", async () => {
    const counter = countOnce("click")
    try {
      await afterMicrotasks()
      expect(counter.committed()).toBe(true)
    } finally {
      counter.unmount()
    }
  })

  it("leaves a mouse move's update to the scheduler", async () => {
    const counter = countOnce("mouseMove")
    try {
      await afterMicrotasks()
      expect(counter.committed()).toBe(false)
      await afterTimer()
      expect(counter.committed()).toBe(true)
    } finally {
      counter.unmount()
    }
  })
})
