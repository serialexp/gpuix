/**
 * Chat performance regression. Times mount, scroll draw, and chrome setState.
 * Skip without the native GPU test renderer.
 *
 * Time dispatchScrollWheel(), not a later flush(). The wheel already draws.
 * Overlay stats include the setup flush after reset, so they are logged only.
 *
 * THROTTLE=utility|background|maintenance re-execs under taskpolicy -c.
 * A throttled run logs numbers and skips the default budgets.
 *
 * Run it through `bun run test:perf`: that sets NODE_ENV=production, so React
 * is the build apps ship. React's development build measures and diffs every
 * component it renders for its DevTools tracks, which more than doubles a
 * commit here and says nothing about what users get. Files run one at a time,
 * so two renderers do not compete for the same cores.
 */

import React from 'react'
import { describe, expect, it } from 'vitest'
import {
  createTestRoot,
  hasNativeTestRenderer,
  readMacCpuThrottle,
  type TestRoot,
} from '@gpuix/react/testing'
import { connectTest } from '@gpuix/react/automation'
import { ChatApp } from './chat'

const describeNative = hasNativeTestRenderer ? describe : describe.skip
const throttle = readMacCpuThrottle()
const TURNS = 1_000
const WARMUP = 10
const WHEEL_SAMPLES = 40
const WHEEL_X = 700
const WHEEL_Y = 400

const BUDGET = {
  // `<code>` paints no card, so the chat draws its own wrapper, header strip
  // and language label: three extra host nodes per code turn, about 6ms over
  // 1000 turns. Chrome costs React nodes now, and that is the whole point.
  mountMs: 170,
  idleP95Ms: 8,
  idleMaxMs: 16,
  wheelP95Ms: 8,
  wheelMaxMs: 16,
  sidebarMs: 40,
  // A root-scoped query over 1000 turns, AFTER the first query paid for the
  // collect + fold. Low enough that keying the group cache on the wrong
  // revision (which re-walks every node per keystroke) fails immediately.
  highlightP95Ms: 12,
  highlightMaxMs: 25,
}

function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0
  const index = Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1)
  return sorted[Math.max(0, index)]!
}

function summarize(samples: number[]) {
  const sorted = [...samples].sort((a, b) => a - b)
  return {
    n: samples.length,
    p50: percentile(sorted, 50),
    p95: percentile(sorted, 95),
    max: sorted.at(-1) ?? 0,
  }
}

function report(label: string, samples: number[]) {
  const stats = summarize(samples)
  const clamp = throttle ?? 'off'
  console.log(
    `[chat.perf] ${label} throttle=${clamp} n=${stats.n} ` +
      `p50=${stats.p50.toFixed(2)}ms p95=${stats.p95.toFixed(2)}ms max=${stats.max.toFixed(2)}ms`,
  )
  return stats
}

function expectBudget(args: {
  label: string
  samples: number[]
  p95Max: number
  maxMax: number
}) {
  const stats = report(args.label, args.samples)
  if (throttle) return stats
  expect(stats.p95, `${args.label} p95 ${stats.p95.toFixed(2)}ms exceeds ${args.p95Max}ms`).toBeLessThan(
    args.p95Max,
  )
  expect(stats.max, `${args.label} max ${stats.max.toFixed(2)}ms exceeds ${args.maxMax}ms`).toBeLessThan(
    args.maxMax,
  )
  return stats
}

function sampleFlushes(args: {
  renderer: TestRoot['renderer']
  count: number
  beforeFlush?: (index: number) => void
}): number[] {
  const samples: number[] = []
  for (let i = 0; i < args.count; i++) {
    args.beforeFlush?.(i)
    const start = performance.now()
    args.renderer.flush()
    samples.push(performance.now() - start)
  }
  return samples
}

it('rejects an unknown THROTTLE value', () => {
  const previous = process.env.THROTTLE
  process.env.THROTTLE = 'nope'
  try {
    expect(() => readMacCpuThrottle()).toThrow(/utility, background, or maintenance/)
  } finally {
    if (previous === undefined) delete process.env.THROTTLE
    else process.env.THROTTLE = previous
  }
})

describeNative('chat performance', () => {
  it('mounts 1000 turns under budget', () => {
    const { render } = createTestRoot()
    const start = performance.now()
    render(<ChatApp turnCount={TURNS} includeSafeMdx />)
    const mountMs = performance.now() - start
    console.log(`[chat.perf] mount throttle=${throttle ?? 'off'} ${mountMs.toFixed(1)}ms`)
    if (!throttle) {
      expect(mountMs, `mount ${mountMs.toFixed(1)}ms exceeds ${BUDGET.mountMs}ms`).toBeLessThan(
        BUDGET.mountMs,
      )
    }
  }, 60_000)

  it('keeps idle flush and wheel draw under budget', () => {
    const { render, renderer } = createTestRoot()
    render(<ChatApp turnCount={TURNS} includeSafeMdx />)

    sampleFlushes({ renderer, count: WARMUP })
    expectBudget({
      label: 'idle flush',
      samples: sampleFlushes({ renderer, count: WHEEL_SAMPLES }),
      p95Max: BUDGET.idleP95Ms,
      maxMax: BUDGET.idleMaxMs,
    })

    for (let i = 0; i < WARMUP; i++) {
      renderer.dispatchScrollWheel(WHEEL_X, WHEEL_Y, 0, i % 2 === 0 ? -160 : 160)
    }
    renderer.resetDebugFrameOverlayStats()
    renderer.flush()

    const wheel: number[] = []
    for (let i = 0; i < WHEEL_SAMPLES; i++) {
      const start = performance.now()
      renderer.dispatchScrollWheel(WHEEL_X, WHEEL_Y, 0, -160)
      wheel.push(performance.now() - start)
    }
    expectBudget({
      label: 'wheel',
      samples: wheel,
      p95Max: BUDGET.wheelP95Ms,
      maxMax: BUDGET.wheelMaxMs,
    })

    const overlay = renderer.getDebugFrameOverlayStats()
    console.log(
      `[chat.perf] overlay p90=${overlay.p90Ms?.toFixed(2)}ms max=${overlay.maxMs?.toFixed(2)}ms samples=${overlay.samples}`,
    )
    expect(overlay.samples).toBeGreaterThan(0)
  }, 60_000)

  // A COARSE budget for the find-bar path, nothing more.
  //
  // It does NOT prove the two-level cache works. Measured: breaking the cache
  // (keying the group list on `subtree_revision` instead of `search_revision`)
  // moves a keystroke from 1.9ms to 2.7ms here, well inside any sane budget,
  // because most of this chat's text lives in native `<code>` / `<diff>` /
  // `<markdown>` props rather than retained `<text>` nodes.
  //
  // The proof is `highlight_cache_tests` in `packages/native/src/renderer.rs`,
  // which compares Arc identity and fails outright when the cache is broken.
  it('keeps a highlight query change under budget', () => {
    const { render, renderer } = createTestRoot()
    const app = (query: string, activeIndex = 0) => (
      <div style={{ flexGrow: 1, flexShrink: 1, flexBasis: 0 }} highlight={query ? { query, activeIndex } : null}>
        <ChatApp turnCount={TURNS} includeSafeMdx />
      </div>
    )
    render(app(''))
    sampleFlushes({ renderer, count: WARMUP })

    // First query: collects and folds the subtree once. Not measured.
    render(app('p'))
    // So are a few more, which the engine needs to compile the commit path.
    for (let i = 0; i < WARMUP; i++) render(app(i % 2 === 0 ? 'pe' : 'per'))

    const word = 'performance of the renderer'
    const keystrokes: number[] = []
    for (let i = 2; i <= word.length; i++) {
      const start = performance.now()
      render(app(word.slice(0, i)))
      keystrokes.push(performance.now() - start)
    }
    const keystroke = expectBudget({
      label: 'highlight keystroke',
      samples: keystrokes,
      p95Max: BUDGET.highlightP95Ms,
      maxMax: BUDGET.highlightMaxMs,
    })

    // Moving the find cursor changes no text and no matcher, so it must never
    // cost more than a keystroke.
    const cursor: number[] = []
    for (let i = 0; i < 20; i++) {
      const start = performance.now()
      render(app(word, i))
      cursor.push(performance.now() - start)
    }
    const stats = report('highlight cursor', cursor)
    if (throttle) return
    expect(
      stats.p50,
      `cursor move ${stats.p50.toFixed(2)}ms should not exceed a keystroke ` +
        `(${keystroke.p50.toFixed(2)}ms); it must not rescan text`,
    ).toBeLessThan(Math.max(keystroke.p50 * 2, 4))
  }, 60_000)

  it('keeps a sidebar click under budget', async () => {
    const { render, renderer } = createTestRoot()
    render(<ChatApp turnCount={TURNS} includeSafeMdx />)
    const app = await connectTest(renderer)
    await app.getByTestId('sidebar-collapse').waitFor()
    await app.clock.pause()

    const samples: number[] = []
    for (let i = 0; i < 8; i++) {
      const testId = i % 2 === 0 ? 'sidebar-collapse' : 'sidebar-expand'
      const start = performance.now()
      await app.getByTestId(testId).click()
      samples.push(performance.now() - start)
      await app.clock.fastForward(200)
    }
    await app.clock.resume()
    const stats = report('sidebar click', samples)
    if (!throttle) {
      expect(
        stats.max,
        `sidebar click ${stats.max.toFixed(1)}ms exceeds ${BUDGET.sidebarMs}ms`,
      ).toBeLessThan(BUDGET.sidebarMs)
    }
  }, 60_000)
})
