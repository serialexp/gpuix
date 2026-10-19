/**
 * Loops one chat interaction so a sampling profiler has something to look at.
 *
 *   MODE=highlight bun profile-chat.tsx &
 *   sample $! 6 -file /tmp/chat.txt
 *
 * MODE=highlight   commit + frame for each query of a find-bar keystroke run
 * MODE=frame       the native frame alone, with nothing changed
 * MODE=commit      React's commit alone for each query
 * MODE=mount       mounts the 1000-turn chat from scratch, over and over
 * MODE=sidebar     clicks the sidebar toggle through the automation client
 *
 * QUERY=...        the find query that is active before the loop starts, 'p' by
 *                  default; set it empty to measure a chat with no search
 *
 * Wall time is noisy on a busy machine, so every mode also reports the CPU time
 * the process spent per iteration, which load on other processes does not move.
 */

import React from 'react'
import { flushSync } from '@gpuix/react'
import { createTestRoot } from '@gpuix/react/testing'
import { connectTest } from '@gpuix/react/automation'
import { ChatApp } from './chat'

const mode = process.env.MODE ?? 'highlight'
const seconds = Number(process.env.SECONDS ?? '12')
const TURNS = 1_000
const word = 'performance of the renderer'

const app = (query: string, activeIndex = 0) => (
  <div style={{ flexGrow: 1, flexShrink: 1, flexBasis: 0 }} highlight={query ? { query, activeIndex } : null}>
    <ChatApp turnCount={TURNS} includeSafeMdx />
  </div>
)

function cpuMs() {
  const usage = process.cpuUsage()
  return (usage.user + usage.system) / 1000
}

let root = createTestRoot()
root.render(app(''))
for (let i = 0; i < 10; i++) root.renderer.flush()
root.render(app(process.env.QUERY ?? 'p'))

const automation = mode === 'sidebar' ? await connectTest(root.renderer) : undefined
if (automation) {
  await automation.getByTestId('sidebar-collapse').waitFor()
  await automation.clock.pause()
}

const end = performance.now() + seconds * 1000
let iterations = 0
let wall = 0
let cpu = 0
let step = 2
while (performance.now() < end) {
  const wallStart = performance.now()
  const cpuStart = cpuMs()
  if (automation) {
    const testId = iterations % 2 === 0 ? 'sidebar-collapse' : 'sidebar-expand'
    await automation.getByTestId(testId).click()
    await automation.clock.fastForward(200)
  } else if (mode === 'mount') {
    root.unmount()
    root = createTestRoot()
    root.render(app(''))
  } else if (mode === 'frame') {
    root.renderer.flush()
  } else {
    const query = word.slice(0, step)
    step = step >= word.length ? 2 : step + 1
    if (mode === 'commit') flushSync(() => root.root.render(app(query)))
    else root.render(app(query))
  }
  wall += performance.now() - wallStart
  cpu += cpuMs() - cpuStart
  iterations++
}
console.log(
  JSON.stringify({
    mode,
    iterations,
    wallMs: +(wall / iterations).toFixed(2),
    cpuMs: +(cpu / iterations).toFixed(2),
  }),
)
process.exit(0)
