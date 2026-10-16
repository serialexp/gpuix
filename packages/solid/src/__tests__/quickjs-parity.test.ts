/// Solid apps under `gpx` behave as they do under Bun: each scenario runs one
/// app from `fixtures/parity` both ways, drives it through the same native
/// window events, and compares the trees it left after each step. Under Bun
/// the preload compiles the fixtures and Solid runs its development build;
/// `gpx` compiles them itself and embeds Solid's production build.
///
/// Needs a native build with the `quickjs` feature
/// (`bun run build:quickjs` in packages/native), and skips without one. The
/// Bun side uses `@gpuix/solid` as built (`bun run build`), as the fixtures'
/// compiled JSX imports it.

import { mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import { createComponent, type Component } from "solid-js"
import { afterEach, describe, expect, it } from "bun:test"
import { render, resetRender, type RenderOptions } from "@gpuix/solid"
import { hasQuickJsTestRenderer, TestRenderer } from "../testing.js"

const FIXTURES = fileURLToPath(new URL("./fixtures/parity/", import.meta.url))
const WIDTH = 480
const HEIGHT = 360
/** Rounds of microtasks, effects and zero-delay timers `settle` allows. */
const SETTLE_ROUNDS = 20
/** Zero-delay timers QuickJS runs in a round, as `gpx` runs at startup. */
const TIMER_ROUNDS = 1_000

interface Fixture {
  App: Component
  options?: RenderOptions
}

/** What a scenario's steps do to the app, the same way in both engines. */
interface Driver {
  renderer: TestRenderer
  /** Let the app finish what the last step started. */
  settle(): Promise<void>
  /** Record the tree, to compare with the other engine's. */
  snapshot(label: string): void
}

interface Scenario {
  name: string
  /** The fixture's file name in `fixtures/parity`. */
  app: string
  steps(driver: Driver): Promise<void>
}

type Snapshot = { label: string; tree: unknown; text: string[]; focused: string | null }

function testIdOf(renderer: TestRenderer, id: number | null): string | null {
  if (id == null) return null
  return renderer.getElement(id)?.testId ?? `#${id}`
}

function centerOf(renderer: TestRenderer, testId: string): [number, number] {
  const element = renderer.findByTestId(testId)
  if (!element) throw new Error(`no element has testId ${testId}`)
  const bounds = renderer.getElementBounds(element.id)
  if (!bounds) throw new Error(`${testId} wasn't painted`)
  return [bounds.x + bounds.width / 2, bounds.y + bounds.height / 2]
}

async function click(driver: Driver, testId: string): Promise<void> {
  const [x, y] = centerOf(driver.renderer, testId)
  driver.renderer.nativeSimulateClick(x, y)
  await driver.settle()
}

async function keys(driver: Driver, keystrokes: string): Promise<void> {
  driver.renderer.simulateKeystrokes(keystrokes)
  await driver.settle()
}

async function typeInto(driver: Driver, testId: string, keystrokes: string): Promise<void> {
  const element = driver.renderer.findByTestId(testId)
  if (!element) throw new Error(`no element has testId ${testId}`)
  driver.renderer.nativeSimulateKeystrokes(element.id, keystrokes)
  await driver.settle()
}

const nextTask = () => new Promise<void>((resolve) => setTimeout(resolve, 0))

function driver(renderer: TestRenderer, runTimers: () => Promise<void>, snapshots: Snapshot[]): Driver {
  return {
    renderer,
    async settle() {
      for (let round = 0; round < SETTLE_ROUNDS; round++) {
        await runTimers()
        renderer.dispatchNativeEvents()
        renderer.flush()
      }
    },
    snapshot(label) {
      snapshots.push({
        label,
        tree: renderer.toJSON(),
        text: renderer.getAllText(),
        focused: testIdOf(renderer, renderer.getFocusedElementId()),
      })
    },
  }
}

async function runInBun(scenario: Scenario): Promise<Snapshot[]> {
  const fixture = (await import(join(FIXTURES, scenario.app))) as Fixture
  const renderer = new TestRenderer({ width: WIDTH, height: HEIGHT })
  const snapshots: Snapshot[] = []
  try {
    render(() => createComponent(fixture.App, {}), { ...fixture.options, renderer })
    const bun = driver(renderer, nextTask, snapshots)
    await bun.settle()
    await scenario.steps(bun)
  } finally {
    resetRender()
  }
  return snapshots
}

async function runInQuickJs(scenario: Scenario, directory: string): Promise<Snapshot[]> {
  // The app's own entry, as a `gpx` app has one. It imports `@gpuix/solid`,
  // so it compiles as Solid, and the package's tsconfig names
  // `@gpuix/solid` for the fixtures.
  const entry = join(directory, "main.js")
  writeFileSync(
    entry,
    [
      `import { createComponent } from "solid-js"`,
      `import { render } from "@gpuix/solid"`,
      `import * as fixture from ${JSON.stringify(join(FIXTURES, scenario.app))}`,
      `render(() => createComponent(fixture.App, {}), fixture.options)`,
    ].join("\n")
  )
  const renderer = new TestRenderer({ width: WIDTH, height: HEIGHT })
  renderer.loadJs(entry)
  const snapshots: Snapshot[] = []
  const quickJs = driver(
    renderer,
    async () => {
      for (let round = 0; round < TIMER_ROUNDS; round++) {
        if (renderer.runJsTimers() !== 0) break
      }
    },
    snapshots
  )
  await quickJs.settle()
  await scenario.steps(quickJs)
  return snapshots
}

const SCENARIOS: Scenario[] = [
  {
    name: "a click updates a signal and restyles",
    app: "counter.tsx",
    async steps(app) {
      app.snapshot("mounted")
      await click(app, "increment")
      app.snapshot("one click")
      await click(app, "increment")
      await click(app, "increment")
      app.snapshot("three clicks")
    },
  },
  {
    name: "keyed rows move, insert and remove",
    app: "keyed-list.tsx",
    async steps(app) {
      app.snapshot("mounted")
      await click(app, "reverse")
      app.snapshot("reversed")
      await click(app, "insert")
      app.snapshot("inserted")
      await click(app, "remove")
      await click(app, "remove")
      app.snapshot("removed two")
      await click(app, "reverse")
      app.snapshot("reversed again")
    },
  },
  {
    name: "effects, their timers and their cleanups run in order",
    app: "effects.tsx",
    async steps(app) {
      app.snapshot("mounted")
      await click(app, "toggle")
      app.snapshot("child unmounted")
      await click(app, "toggle")
      app.snapshot("child mounted again")
    },
  },
  {
    name: "a controlled input takes typed text",
    app: "text-input.tsx",
    async steps(app) {
      app.snapshot("mounted")
      await typeInto(app, "input", "h i")
      app.snapshot("typed")
      await typeInto(app, "input", "backspace")
      app.snapshot("deleted one")
    },
  },
  {
    name: "focus moves with tab, and keys reach the focused element",
    app: "keyboard.tsx",
    async steps(app) {
      app.snapshot("mounted")
      await keys(app, "a")
      await keys(app, "tab")
      app.snapshot("tabbed")
      await keys(app, "b enter")
      await keys(app, "shift-tab")
      app.snapshot("tabbed back")
    },
  },
  {
    name: "window methods answer through the app's renderer",
    app: "window.tsx",
    async steps(app) {
      app.snapshot("mounted")
      await click(app, "check")
      app.snapshot("focused next")
    },
  },
  {
    name: "the bundled Select opens, highlights and chooses",
    app: "select.tsx",
    async steps(app) {
      app.snapshot("mounted")
      await click(app, "trigger")
      app.snapshot("open")
      await keys(app, "down")
      app.snapshot("highlight moved")
      await click(app, "cherry")
      app.snapshot("chose cherry")
    },
  },
]

describe.skipIf(!hasQuickJsTestRenderer)("Solid under gpx's QuickJS matches Solid under Bun", () => {
  let directory: string | undefined

  afterEach(() => {
    if (directory) rmSync(directory, { recursive: true, force: true })
    directory = undefined
  })

  for (const scenario of SCENARIOS) {
    it(scenario.name, async () => {
      directory = mkdtempSync(join(tmpdir(), "gpuix-parity-"))
      const bun = await runInBun(scenario)
      const quickJs = await runInQuickJs(scenario, directory)
      expect(quickJs.map((snapshot) => snapshot.label)).toEqual(
        bun.map((snapshot) => snapshot.label)
      )
      for (const [index, expected] of bun.entries()) {
        expect({ label: expected.label, ...quickJs[index] }).toEqual(expected)
      }
    })
  }
})
