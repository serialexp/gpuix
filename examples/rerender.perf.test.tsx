/**
 * Re-render cost when nothing visible changed. This is the shape of a chat
 * client: a parent holds state (composer text, a streaming message, a clock),
 * every row below it re-renders, and each row builds its `style` object and its
 * handlers from scratch. React cannot skip them, so what matters is how cheap
 * the renderer makes "the same thing again".
 *
 * Reports the mutations that cross into Rust per update next to the time, since
 * an update that sends nothing cannot cost a frame. Skip without the native GPU
 * test renderer. Excluded from `bun run test` by the `*.perf.test.tsx` glob.
 */

import React from 'react'
import { describe, expect, it, vi } from 'vitest'
import { flushSync } from '@gpuix/react'
import { createTestRoot, hasNativeTestRenderer, type TestRoot } from '@gpuix/react/testing'
import { App } from './rerender-fixture'

const describeNative = hasNativeTestRenderer ? describe : describe.skip

const WARMUP = 5
const SAMPLES = 30

function measure(
  root: TestRoot,
  next: (index: number) => React.ReactNode,
): { ms: number[]; frameMs: number[]; ops: number; bytes: number } {
  const { renderer } = root
  let ops = 0
  let bytes = 0
  const original = renderer.applyBatch.bind(renderer)
  const applyBatch = vi.spyOn(renderer, 'applyBatch').mockImplementation((json: string) => {
    bytes += json.length
    ops += (JSON.parse(json) as unknown[]).length
    return original(json)
  })
  // `ms` is React rendering and committing, which includes `applyBatch`.
  // `frameMs` is the native frame that follows. A real window only runs one when
  // a batch left something dirty, but the test renderer always does. Not
  // `root.render()`, which flushes a frame of its own and would count it twice.
  const ms: number[] = []
  const frameMs: number[] = []
  try {
    for (let i = 0; i < WARMUP + SAMPLES; i++) {
      if (i === WARMUP) {
        ops = 0
        bytes = 0
      }
      const start = performance.now()
      flushSync(() => root.root.render(next(i)))
      const committed = performance.now()
      renderer.flush()
      if (i >= WARMUP) {
        ms.push(committed - start)
        frameMs.push(performance.now() - committed)
      }
    }
  } finally {
    applyBatch.mockRestore()
  }
  return { ms, frameMs, ops: ops / SAMPLES, bytes: bytes / SAMPLES }
}

function p(samples: number[], q: number) {
  const sorted = [...samples].sort((a, b) => a - b)
  return sorted[Math.min(sorted.length - 1, Math.ceil(q * sorted.length) - 1)]!
}

function report(label: string, result: ReturnType<typeof measure>) {
  console.log(
    `[rerender.perf] ${label}: commit p50=${p(result.ms, 0.5).toFixed(2)}ms p95=${p(result.ms, 0.95).toFixed(2)}ms ` +
      `frame p50=${p(result.frameMs, 0.5).toFixed(2)}ms p95=${p(result.frameMs, 0.95).toFixed(2)}ms ` +
      `ops/update=${result.ops.toFixed(0)} bytes/update=${result.bytes.toFixed(0)}`,
  )
}

describeNative('re-render with nothing visibly changed', () => {
  it('sends only the streaming row when the whole list re-renders', () => {
    const root = createTestRoot({ width: 900, height: 700 })
    root.render(<App tick={0} streaming="" />)
    let text = ''
    const result = measure(root, (index) => {
      text += 'word '
      return <App tick={index + 1} streaming={text} />
    })
    report('streaming row, whole list re-renders', result)
    // The `tick` text and the streaming row's text are the only two changes. Each
    // costs one op; anything near the row count means rows resend what they have.
    expect(result.ops).toBeLessThan(10)
  }, 120_000)

  it('sends one op when only a header changes', () => {
    const root = createTestRoot({ width: 900, height: 700 })
    root.render(<App tick={0} streaming="done" />)
    const result = measure(root, (index) => <App tick={index + 1} streaming="done" />)
    report('header text only, whole list re-renders', result)
    expect(result.ops).toBeLessThan(5)
  }, 120_000)
})
