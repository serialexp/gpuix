import { Suspense } from "react"
import { describe, expect, it, vi } from "vitest"
import { handleGpuixEvent } from "../reconciler/event-registry.js"
import { createRoot, flushSync } from "../reconciler/reconciler.js"
import { createTestRoot, hasNativeTestRenderer, TestRenderer } from "../testing.js"

const describeNative = hasNativeTestRenderer ? describe : describe.skip

describeNative("mutation lifecycle", () => {
  it("renders and updates through Rust snapshot reconciliation", () => {
    const { render, renderer, unmount } = createTestRoot({ transport: "snapshot" })

    try {
      render(
        <div style={{ display: "flex", flexDirection: "column" }}>
          {["a", "b", "c"].map((value) => (
            <text key={value}>{value}</text>
          ))}
        </div>
      )
      expect(renderer.getAllText()).toEqual(["a", "b", "c"])

      render(
        <div style={{ display: "flex", flexDirection: "column" }}>
          {["c", "b", "a"].map((value) => (
            <text key={value} style={{ color: value === "b" ? "red" : "white" }}>
              {value}
            </text>
          ))}
        </div>
      )
      expect(renderer.getAllText()).toEqual(["c", "b", "a"])
    } finally {
      unmount()
    }
    expect(renderer.getRetainedElementCount()).toBe(0)
  })

  it("does not paint host nodes from an abandoned Suspense render", () => {
    const { render, renderer, unmount } = createTestRoot()
    const pending = new Promise<never>(() => {})

    function Suspend(): never {
      throw pending
    }

    try {
      render(
        <Suspense fallback={<text>fallback</text>}>
          <div>
            <text>abandoned</text>
          </div>
          <Suspend />
        </Suspense>
      )

      expect(renderer.getPaintedText()).toEqual(["fallback"])
    } finally {
      unmount()
    }
  })

  it("keeps unchanged event handlers registered across renders", () => {
    const { render, renderer, unmount } = createTestRoot()
    const onClick = vi.fn()
    const clickable = (
      <div style={{ width: 100, height: 100 }} onClick={onClick}>
        click
      </div>
    )

    try {
      render(clickable)
      renderer.nativeSimulateClick(10, 10)
      render(clickable)
      renderer.nativeSimulateClick(10, 10)

      expect(onClick).toHaveBeenCalledTimes(2)
    } finally {
      unmount()
    }
  })

  it("keeps element ids and click handlers isolated across live roots", () => {
    const a = createTestRoot()
    const b = createTestRoot()
    const onA = vi.fn()
    const onB = vi.fn()

    try {
      a.render(
        <div style={{ width: 100, height: 100 }} onClick={onA}>
          a
        </div>
      )
      b.render(
        <div style={{ width: 100, height: 100 }} onClick={onB}>
          b
        </div>
      )

      expect(a.renderer.findByType("text")[0]?.id).toBe(1)
      expect(b.renderer.findByType("text")[0]?.id).toBe(1)
      expect(a.renderer.getRoot()?.id).toBe(2)
      expect(b.renderer.getRoot()?.id).toBe(2)

      const click = { elementId: 2, eventType: "click" }
      handleGpuixEvent(click, a.renderer)
      expect(onA).toHaveBeenCalledTimes(1)
      expect(onB).not.toHaveBeenCalled()

      handleGpuixEvent(click, b.renderer)
      expect(onA).toHaveBeenCalledTimes(1)
      expect(onB).toHaveBeenCalledTimes(1)
    } finally {
      a.unmount()
      b.unmount()
    }
  })

  it("does not give a remounted root the old element ids", () => {
    const renderer = new TestRenderer()
    const first = createRoot(renderer)
    const onFirst = vi.fn()
    const onSecond = vi.fn()
    const tree = (onClick: () => void) => (
      <div style={{ width: 100, height: 100 }} onClick={onClick}>
        row
      </div>
    )

    flushSync(() => first.render(tree(onFirst)))
    renderer.flush()
    const firstRootId = renderer.getRoot()?.id
    expect(firstRootId).toBe(2)
    first.unmount()

    const second = createRoot(renderer)
    try {
      flushSync(() => second.render(tree(onSecond)))
      renderer.flush()
      expect(renderer.getRoot()?.id).not.toBe(firstRootId)

      handleGpuixEvent({ elementId: firstRootId!, eventType: "click" }, renderer)
      expect(onSecond).not.toHaveBeenCalled()
    } finally {
      second.unmount()
    }
  })

  // React calls `detachDeletedInstance` only for host components, never for a
  // host text node, so a removed string used to stay in the retained tree with
  // no parent and no way back to it.
  it("frees a removed text node instead of leaking it", () => {
    const { render, renderer, unmount } = createTestRoot()

    try {
      render(<div style={{ width: 100, height: 100 }}>{"hello"}</div>)
      const withText = renderer.getRetainedElementCount()

      render(<div style={{ width: 100, height: 100 }}>{null}</div>)
      expect(renderer.getAllText()).toEqual([])
      expect(renderer.getRetainedElementCount()).toBe(withText - 1)

      // A whole removed subtree frees every node in it, not just its root.
      render(
        <div style={{ width: 100, height: 100 }}>
          <div>
            <text>deep</text>
          </div>
        </div>
      )
      const withSubtree = renderer.getRetainedElementCount()
      expect(withSubtree).toBe(withText + 2)

      render(<div style={{ width: 100, height: 100 }}>{null}</div>)
      expect(renderer.getRetainedElementCount()).toBe(withText - 1)
    } finally {
      unmount()
    }
  })

  it("refuses a second simultaneous root on one renderer", () => {
    const renderer = new TestRenderer()
    const onFirstKey = vi.fn()
    const first = createRoot(renderer, { onKeyDown: onFirstKey })
    const onFirst = vi.fn()

    try {
      flushSync(() =>
        first.render(
          <div style={{ width: 100, height: 100 }} onClick={onFirst}>
            first
          </div>
        )
      )
      renderer.flush()

      expect(() => createRoot(renderer)).toThrowErrorMatchingInlineSnapshot(
        `[Error: This renderer already drives a mounted GPUIX root. One renderer owns one window, one native root id, and one event map, so a second root would silently take both over. Unmount the first root first.]`
      )

      // The rejected root must not have disturbed the live one.
      handleGpuixEvent({ elementId: renderer.getRoot()!.id, eventType: "click" }, renderer)
      expect(onFirst).toHaveBeenCalledTimes(1)
      renderer.simulateKeystrokes("tab")
      expect(onFirstKey).toHaveBeenCalledTimes(1)
    } finally {
      first.unmount()
    }
  })

  it("allows a new root once the previous one unmounts", () => {
    const renderer = new TestRenderer()
    const first = createRoot(renderer)
    flushSync(() => first.render(<text>first</text>))
    renderer.flush()
    first.unmount()

    const second = createRoot(renderer)
    const onSecond = vi.fn()
    try {
      flushSync(() =>
        second.render(
          <div style={{ width: 100, height: 100 }} onClick={onSecond}>
            second
          </div>
        )
      )
      renderer.flush()
      handleGpuixEvent({ elementId: renderer.getRoot()!.id, eventType: "click" }, renderer)
      expect(onSecond).toHaveBeenCalledTimes(1)
    } finally {
      second.unmount()
    }
  })
})
