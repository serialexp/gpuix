import { createSignal, For, Show } from "solid-js"
import { describe, expect, it } from "bun:test"
import { AnimatePresence, motion } from "../index.js"
import { createTestRoot, hasNativeTestRenderer } from "../testing.js"

/** Solid 2.0 applies writes on a microtask, and the native tree takes the
 *  mutations on the next one; a task later both have run. */
const nextTask = () => new Promise<void>((resolve) => setTimeout(resolve, 0))

describe.skipIf(!hasNativeTestRenderer)("Solid renderer", () => {
  it("mounts and applies signal text and property updates", () => {
    const [count, setCount] = createSignal(0)
    const app = createTestRoot()
    app.render(() => (
      <div testId="root" style={{ padding: count() }}>
        <text>count {count()}</text>
      </div>
    ))
    expect(app.renderer.getAllText()).toEqual(["count ", "0"])
    app.flushSync(() => setCount(2))
    expect(app.renderer.getAllText()).toEqual(["count ", "2"])
    expect(app.renderer.findByTestId("root")?.style.padding).toBe(2)
  })

  it("flushes signal updates outside an event boundary", async () => {
    const [count, setCount] = createSignal(0)
    const app = createTestRoot()
    app.render(() => <text>{count()}</text>)

    setCount(1)
    await nextTask()

    expect(app.renderer.getAllText()).toEqual(["1"])
  })

  it("routes real native events and replaces listeners", () => {
    const [count, setCount] = createSignal(0)
    const [alternate, setAlternate] = createSignal(false)
    const app = createTestRoot()
    app.render(() => (
      <div
        testId="button"
        onClick={() => setCount((value) => value + (alternate() ? 10 : 1))}
        style={{ width: 100, height: 40, backgroundColor: "#333333" }}
      >
        <text>{count()}</text>
      </div>
    ))
    const button = app.renderer.findByTestId("button")!
    expect([...button.events]).toContain("click")
    const bounds = app.renderer.getElementBounds(button.id)!
    app.renderer.nativeSimulateClick(
      bounds.x + bounds.width / 2,
      bounds.y + bounds.height / 2
    )
    expect(count()).toBe(1)
    expect(app.renderer.getAllText()).toEqual(["1"])
    app.flushSync(() => setAlternate(true))
    app.renderer.nativeSimulateClick(
      bounds.x + bounds.width / 2,
      bounds.y + bounds.height / 2
    )
    expect(app.renderer.getAllText()).toEqual(["11"])
  })

  it("inserts, removes, and reorders keyed nodes", () => {
    const [show, setShow] = createSignal(true)
    const [items, setItems] = createSignal(["a", "b", "c"])
    const app = createTestRoot()
    app.render(() => (
      <div>
        <Show when={show()}><text>shown</text></Show>
        <For each={items()}>{(item) => <text>{item}</text>}</For>
      </div>
    ))
    app.flushSync(() => {
      setShow(false)
      setItems(["c", "a"])
    })
    expect(app.renderer.getAllText().filter(Boolean)).toEqual(["c", "a"])
  })

  it("forwards custom elements, virtual lists, and motion", () => {
    const app = createTestRoot()
    app.render(() => (
      <div>
        <input value="hello" placeholder="Type" />
        <markdown source="**bold**" />
        <virtual-list estimatedItemHeight={20}><text>row</text></virtual-list>
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={{ duration: 0.2 }}
          testId="motion"
        />
      </div>
    ))
    expect(app.renderer.findByType("input")).toHaveLength(1)
    expect(app.renderer.findByType("markdown")).toHaveLength(1)
    expect(app.renderer.findByType("virtual-list")).toHaveLength(1)
    expect(app.renderer.findByTestId("motion")?.customProps?.motion).toMatchObject({
      animate: { opacity: 1 },
    })
  })

  it("lays out an empty placeholder as nothing, so it takes no gap", () => {
    const [shown, setShown] = createSignal(false)
    const app = createTestRoot()
    const box = { width: 20, height: 20 }
    app.render(() => (
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <div testId="a" style={box} />
        <Show when={shown()}>
          <div testId="shown" style={box} />
        </Show>
        {null}
        <div testId="b" style={box} />
      </div>
    ))
    const top = (testId: string) =>
      app.renderer.getElementBounds(app.renderer.findByTestId(testId)!.id)!.y
    expect(top("b") - top("a")).toBe(30)

    app.flushSync(() => setShown(true))
    expect(top("shown") - top("a")).toBe(30)
    expect(top("b") - top("a")).toBe(60)
  })

  it("removes listeners and rejects stale events after remount", () => {
    const [enabled, setEnabled] = createSignal(true)
    let clicks = 0
    const app = createTestRoot()
    app.render(() => (
      <div
        testId="target"
        onClick={enabled() ? () => clicks++ : undefined}
        style={{ width: 80, height: 30, backgroundColor: "#333333" }}
      />
    ))
    const first = app.renderer.findByTestId("target")!
    const bounds = app.renderer.getElementBounds(first.id)!
    app.renderer.nativeSimulateClick(bounds.x + 5, bounds.y + 5)
    expect(clicks).toBe(1)

    app.flushSync(() => setEnabled(false))
    expect(app.renderer.findByTestId("target")?.events.has("click")).toBe(false)
    expect(app.root.dispatch({ elementId: first.id, eventType: "click" })).toBe(false)

    app.render(() => <div testId="replacement" />)
    const replacement = app.renderer.findByTestId("replacement")!
    expect(replacement.id).toBeGreaterThan(first.id)
    expect(app.root.dispatch({ elementId: first.id, eventType: "click" })).toBe(false)
  })

  it("keeps an exiting motion node until native completion", () => {
    const [visible, setVisible] = createSignal(true)
    let exits = 0
    const app = createTestRoot()
    app.render(() => (
      <AnimatePresence onExitComplete={() => exits++}>
        <Show when={visible()}>
          <motion.div
            testId="toast"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
          />
        </Show>
      </AnimatePresence>
    ))
    const toast = app.renderer.findByTestId("toast")!
    app.flushSync(() => setVisible(false))
    expect(app.renderer.findByTestId("toast")?.customProps?.motion).toMatchObject({
      animate: { opacity: 0 },
    })
    const generation = Number(
      app.renderer.findByTestId("toast")?.customProps?.motion?.generation
    )
    expect(app.root.dispatch({
      elementId: toast.id,
      eventType: "motionComplete",
      motionGeneration: generation,
    })).toBe(true)
    expect(app.renderer.findByTestId("toast")).toBeUndefined()
    expect(exits).toBe(1)
  })

  it("skips the first render's enter animations with initial={false}", () => {
    const [items, setItems] = createSignal(["a"])
    const app = createTestRoot()
    app.render(() => (
      <AnimatePresence initial={false}>
        <For each={items()}>
          {(item) => (
            <motion.div testId={item} initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} />
          )}
        </For>
      </AnimatePresence>
    ))
    const initial = (id: string) => app.renderer.findByTestId(id)?.customProps?.motion?.initial
    expect(initial("a")).toBe(false)
    app.flushSync(() => setItems(["a", "b"]))
    expect(initial("b")).toEqual({ opacity: 0 })
  })

  it("brings an exiting child back when it returns before its exit ends", () => {
    const [visible, setVisible] = createSignal(true)
    let exits = 0
    const app = createTestRoot()
    app.render(() => {
      // One element that leaves and comes back, not a new one each time.
      const toast = <div testId="toast" motion={{ animate: { opacity: 1 }, exit: { opacity: 0 } }} />
      return (
        <AnimatePresence onExitComplete={() => exits++}>
          {visible() ? toast : null}
        </AnimatePresence>
      )
    })
    const toastMotion = () => app.renderer.findByTestId("toast")?.customProps?.motion
    app.flushSync(() => setVisible(false))
    expect(toastMotion()).toMatchObject({ isExit: true })
    app.flushSync(() => setVisible(true))
    expect(toastMotion()).toMatchObject({ isExit: false, animate: { opacity: 1 } })
    expect(exits).toBe(0)
  })

  it("ignores stale motion completion targets", async () => {
    const [opacity, setOpacity] = createSignal(0)
    let completions = 0
    const app = createTestRoot()
    app.render(() => (
      <motion.div
        testId="motion-target"
        animate={{ opacity: opacity() }}
        onMotionComplete={() => completions++}
      />
    ))
    const target = app.renderer.findByTestId("motion-target")!
    const firstGeneration = Number(target.customProps?.motion?.generation)

    setOpacity(1)
    await nextTask()
    const nextGeneration = Number(
      app.renderer.findByTestId("motion-target")?.customProps?.motion?.generation
    )
    expect(nextGeneration).toBeGreaterThan(firstGeneration)

    expect(app.root.dispatch({
      elementId: target.id,
      eventType: "motionComplete",
      motionGeneration: firstGeneration,
    })).toBe(true)
    expect(completions).toBe(0)

    app.root.dispatch({
      elementId: target.id,
      eventType: "motionComplete",
      motionGeneration: nextGeneration,
    })
    expect(completions).toBe(1)
  })

  it("delivers onLayout with the painted size", () => {
    const sizes: unknown[] = []
    const app = createTestRoot()
    app.render(() => (
      <div
        style={{ width: 90, height: 30 }}
        onLayout={(event) => sizes.push([event.width, event.height])}
      />
    ))
    app.renderer.dispatchNativeEvents()
    expect(sizes).toEqual([[90, 30]])
  })
})
