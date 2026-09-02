/// End-to-end mutation transport benchmark across representative React trees.
///
/// Unlike bench-serialization.ts, which compares codecs on one deliberately
/// huge chat mount, this file varies tree shape and commit type. Every workload
/// goes through the real React reconciler, then through the native napi
/// TestGpuixRenderer.applyBatch entry point.
///
/// Run:
///   bun bench-workloads.tsx
///   ITERATIONS=9 CHAT_TURNS=10000 bun bench-workloads.tsx

import React from 'react'
import { createRequire } from 'node:module'
import { createRoot, flushSync } from '@gpuix/react'
import type { NativeRenderer } from '@gpuix/react'
import { ChatApp } from './chat'

type Op = unknown[]

interface NativeBatchRenderer {
  applyBatch(json: string): number[]
  getRetainedElementCount(): number
}

interface NativeBinding {
  TestGpuixRenderer?: new (width?: number, height?: number) => NativeBatchRenderer
}

interface MutationWorkload {
  name: string
  kind: 'mount' | 'update'
  description: string
  setup: Op[][]
  target: Op[]
  reactRenderMs: number
  reactCommitMs: number
}

interface WorkloadSpec {
  name: string
  description: string
  initial: React.ReactNode
  update?: React.ReactNode
}

class CaptureRenderer implements NativeRenderer {
  batches: string[] = []

  applyBatch(json: string): number[] {
    this.batches.push(json)
    return []
  }

  getWindowSize(): { width: number; height: number } {
    return { width: 1440, height: 900 }
  }
}

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[Math.floor(sorted.length / 2)]!
}

function capture(
  name: string,
  description: string,
  initial: React.ReactNode,
  update?: React.ReactNode,
): MutationWorkload {
  const renderer = new CaptureRenderer()
  const root = createRoot(renderer)
  let reactRenderMs = 0
  const onRender: React.ProfilerOnRenderCallback = (
    _id,
    _phase,
    actualDuration,
  ) => {
    reactRenderMs = actualDuration
  }
  const profiled = (node: React.ReactNode) => (
    <React.Profiler id={name} onRender={onRender}>
      {node}
    </React.Profiler>
  )

  let start = performance.now()
  flushSync(() => root.render(profiled(initial)))
  const mountCommitMs = performance.now() - start
  const setup = renderer.batches.map((batch) => JSON.parse(batch) as Op[])
  let reactCommitMs = mountCommitMs
  if (update !== undefined) {
    start = performance.now()
    flushSync(() => root.render(profiled(update)))
    reactCommitMs = performance.now() - start
  }

  const target =
    update === undefined
      ? setup[setup.length - 1]!
      : JSON.parse(renderer.batches[renderer.batches.length - 1]!) as Op[]
  const setupForNative = update === undefined ? [] : setup
  root.unmount()

  return {
    name,
    kind: update === undefined ? 'mount' : 'update',
    description,
    setup: setupForNative,
    target,
    reactRenderMs,
    reactCommitMs,
  }
}

const NOOP = () => {}
const ROW_STYLE = {
  display: 'flex',
  flexDirection: 'row',
  alignItems: 'center',
  gap: 8,
  height: 32,
  paddingLeft: 8,
  paddingRight: 8,
} as const

function DeepBranch({ depth, seed }: { depth: number; seed: number }) {
  const tone = (seed * 37 + depth * 19) % 255
  if (depth === 0) {
    return <text style={{ color: `rgb(${tone}, 180, 210)`, fontSize: 12 }}>leaf-{seed}</text>
  }
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: depth % 2 === 0 ? 'row' : 'column',
        gap: (seed % 3) + 1,
        padding: depth % 3,
        borderWidth: seed % 5 === 0 ? 1 : 0,
        borderColor: `rgb(${tone}, ${255 - tone}, 120)`,
      }}
    >
      <DeepBranch depth={depth - 1} seed={seed * 3 + 1} />
      <DeepBranch depth={depth - 1} seed={seed * 3 + 2} />
      <DeepBranch depth={depth - 1} seed={seed * 3 + 3} />
    </div>
  )
}

function InteractiveDashboard({ cards = 1_000, theme = 0 }: { cards?: number; theme?: number }) {
  const accent = theme === 0 ? '#6ea8fe' : '#e2795b'
  return (
    <div style={{ display: 'flex', flexDirection: 'column', width: '100%', height: '100%' }}>
      <div style={{ display: 'flex', flexDirection: 'row', gap: 8, height: 48 }}>
        <input value="Search metrics" onChange={NOOP} onSubmit={NOOP} style={{ width: 260 }} />
        {Array.from({ length: 12 }, (_, index) => (
          <div key={index} tabIndex={0} onClick={NOOP} onKeyDown={NOOP} style={{ padding: 8 }}>
            <text style={{ color: accent }}>Filter {index}</text>
          </div>
        ))}
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: 4, gap: 10 }}>
        {Array.from({ length: cards }, (_, index) => (
          <div
            key={index}
            onClick={NOOP}
            onMouseEnter={NOOP}
            onMouseLeave={NOOP}
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: 6,
              padding: 12,
              borderRadius: 8,
              borderWidth: 1,
              borderColor: index % 7 === 0 ? accent : '#30343b',
              backgroundColor: index % 2 === 0 ? '#17191d' : '#1d2025',
            }}
          >
            <text style={{ color: '#d8dee9', fontSize: 13 }}>Metric {index}</text>
            <text style={{ color: accent, fontSize: 22 }}>{(index * 17).toLocaleString()}</text>
            <div style={{ display: 'flex', flexDirection: 'row', gap: 4 }}>
              {Array.from({ length: 3 }, (_, tag) => (
                <text key={tag} style={{ color: '#88909c', fontSize: 11 }}>
                  tag-{(index + tag) % 31}
                </text>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}

const ICON_SOURCE = [
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">',
  '<path fill="#000" d="M3 5h18v14H3z"/>',
  '<path fill="#fff" d="m6 16 4-5 3 3 2-2 3 4z"/>',
  '</svg>',
].join('')

function MediaGrid({ items = 2_000, revision = 0 }: { items?: number; revision?: number }) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: 5, gap: 8 }}>
      {Array.from({ length: items }, (_, index) => (
        <div
          key={index}
          onClick={NOOP}
          style={{ display: 'flex', flexDirection: 'column', gap: 6, padding: 6 }}
        >
          <img
            src={`/fixtures/photos/${revision}/${index}.webp`}
            objectFit="cover"
            style={{ width: 180, height: 110, borderRadius: 6 }}
          />
          <div style={{ display: 'flex', flexDirection: 'row', alignItems: 'center', gap: 6 }}>
            <svg source={ICON_SOURCE} style={{ width: 16, height: 16, color: '#9aa4b2' }} />
            <text style={{ color: '#d8dee9', fontSize: 12 }}>Asset {index}</text>
          </div>
        </div>
      ))}
    </div>
  )
}

function UpdateList({ changed = -1, rows = 10_000 }: { changed?: number; rows?: number }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column' }}>
      {Array.from({ length: rows }, (_, index) => (
        <div key={index} style={ROW_STYLE}>
          <text style={{ color: '#d8dee9' }}>Row {index}</text>
          <text style={{ color: index === changed ? '#e2795b' : '#7f8792' }}>
            {index === changed ? 'changed' : 'stable'}
          </text>
        </div>
      ))}
    </div>
  )
}

function ReorderList({ reversed = false, rows = 5_000 }: { reversed?: boolean; rows?: number }) {
  const indexes = Array.from({ length: rows }, (_, index) => index)
  if (reversed) indexes.reverse()
  return (
    <div style={{ display: 'flex', flexDirection: 'column' }}>
      {indexes.map((index) => (
        <div key={index} style={ROW_STYLE}>
          <text style={{ color: '#d8dee9' }}>Item {index}</text>
        </div>
      ))}
    </div>
  )
}

function opCounts(ops: Op[]): Record<string, number> {
  const counts: Record<string, number> = {}
  for (const op of ops) counts[String(op[0])] = (counts[String(op[0])] ?? 0) + 1
  return counts
}

function rootId(workload: MutationWorkload): number | null {
  for (const batch of [...workload.setup, workload.target]) {
    for (const op of batch) if (op[0] === 'setRoot') return Number(op[1])
  }
  return null
}

interface Timing {
  stringifyMs: number
  nativeMs: number
  endToEndMs: number
}

interface CommitTiming {
  reactRenderMs: number
  wallMs: number
}

function measureNativeCommit(
  renderer: NativeBatchRenderer,
  spec: WorkloadSpec,
  iterations: number,
): CommitTiming {
  const renders: number[] = []
  const walls: number[] = []
  for (let iteration = 0; iteration < iterations; iteration++) {
    const root = createRoot(renderer as NativeRenderer)
    let reactRenderMs = 0
    const onRender: React.ProfilerOnRenderCallback = (
      _id,
      _phase,
      actualDuration,
    ) => {
      reactRenderMs = actualDuration
    }
    const profiled = (node: React.ReactNode) => (
      <React.Profiler id={spec.name} onRender={onRender}>
        {node}
      </React.Profiler>
    )

    if (spec.update !== undefined) {
      flushSync(() => root.render(profiled(spec.initial)))
    }
    const start = performance.now()
    flushSync(() => root.render(profiled(spec.update ?? spec.initial)))
    walls.push(performance.now() - start)
    renders.push(reactRenderMs)
    root.unmount()
    if (renderer.getRetainedElementCount() !== 0) {
      throw new Error(`${spec.name} commit left native elements behind`)
    }
  }
  return { reactRenderMs: median(renders), wallMs: median(walls) }
}

function measureNative(
  renderer: NativeBatchRenderer,
  workload: MutationWorkload,
  iterations: number,
): Timing {
  const payload = JSON.stringify(workload.target)
  const id = rootId(workload)
  const reset = (): void => {
    if (id !== null) renderer.applyBatch(JSON.stringify([['destroyElement', id]]))
    for (const setup of workload.setup) renderer.applyBatch(JSON.stringify(setup))
  }

  for (let iteration = 0; iteration < 3; iteration++) {
    reset()
    renderer.applyBatch(payload)
  }

  const stringify: number[] = []
  const native: number[] = []
  const endToEnd: number[] = []
  for (let iteration = 0; iteration < iterations; iteration++) {
    let start = performance.now()
    JSON.stringify(workload.target)
    stringify.push(performance.now() - start)

    reset()
    start = performance.now()
    renderer.applyBatch(payload)
    native.push(performance.now() - start)

    reset()
    start = performance.now()
    renderer.applyBatch(JSON.stringify(workload.target))
    endToEnd.push(performance.now() - start)
  }

  if (id !== null) renderer.applyBatch(JSON.stringify([['destroyElement', id]]))
  if (renderer.getRetainedElementCount() !== 0) {
    throw new Error(`${workload.name} left native elements behind after cleanup`)
  }

  return {
    stringifyMs: median(stringify),
    nativeMs: median(native),
    endToEndMs: median(endToEnd),
  }
}

function number(value: number): string {
  return value.toLocaleString('en-US')
}

function milliseconds(value: number): string {
  return `${value.toFixed(2)} ms`
}

const iterations = Number(process.env.ITERATIONS ?? 5)
const commitIterations = Number(process.env.COMMIT_ITERATIONS ?? 3)
const chatTurns = Number(process.env.CHAT_TURNS ?? 10_000)

const workloadSpecs: WorkloadSpec[] = [
  {
    name: 'wide chat',
    description: 'One children-mode virtual list retaining every row',
    initial: <ChatApp turnCount={chatTurns} includeSafeMdx />,
  },
  {
    name: 'deep branches',
    description: 'Ternary tree with varied styles and depth 8',
    initial: <DeepBranch depth={8} seed={1} />,
  },
  {
    name: 'interactive dashboard',
    description: '1,000 cards with keyboard, pointer, and input handlers',
    initial: <InteractiveDashboard />,
  },
  {
    name: 'media grid',
    description: '2,000 image paths plus repeated inline SVG source',
    initial: <MediaGrid />,
  },
  {
    name: 'single leaf update',
    description: 'One changed row inside a 10,000-row retained tree',
    initial: <UpdateList />,
    update: <UpdateList changed={5_000} />,
  },
  {
    name: 'broad theme update',
    description: 'Style change across 1,000 interactive cards',
    initial: <InteractiveDashboard theme={0} />,
    update: <InteractiveDashboard theme={1} />,
  },
  {
    name: 'keyed reorder',
    description: 'Reverse 5,000 retained keyed rows',
    initial: <ReorderList />,
    update: <ReorderList reversed />,
  },
]

const workloads: MutationWorkload[] = workloadSpecs.map((spec) =>
  capture(spec.name, spec.description, spec.initial, spec.update),
)

const require = createRequire(import.meta.url)
const native = require('../packages/native/index.js') as NativeBinding
const Renderer = native.TestGpuixRenderer
if (!Renderer) {
  throw new Error(
    'This benchmark needs TestGpuixRenderer. Build packages/native with test-support first.',
  )
}
const renderer = new Renderer(320, 200)

console.log('| workload | kind | ops | JSON | stringify | napi + Rust | combined |')
console.log('|---|---|---:|---:|---:|---:|---:|')
for (const workload of workloads) {
  const timing = measureNative(renderer, workload, iterations)
  const bytes = Buffer.byteLength(JSON.stringify(workload.target))
  console.log(
    `| ${workload.name} | ${workload.kind} | ${number(workload.target.length)} |` +
      ` ${(bytes / 1e6).toFixed(2)} MB | ${milliseconds(timing.stringifyMs)} |` +
      ` ${milliseconds(timing.nativeMs)} | **${milliseconds(timing.endToEndMs)}** |`,
  )
}

console.log('\n## Actual React commit through NAPI\n')
console.log('| workload | React render | commit wall |')
console.log('|---|---:|---:|')
for (const spec of workloadSpecs) {
  const commit = measureNativeCommit(renderer, spec, commitIterations)
  console.log(
    `| ${spec.name} | ${milliseconds(commit.reactRenderMs)} | **${milliseconds(commit.wallMs)}** |`,
  )
}

console.log('\n## Operation mix\n')
console.log('| workload | create | append | style | text | events | custom props | React render | commit wall |')
console.log('|---|---:|---:|---:|---:|---:|---:|---:|---:|')
for (const workload of workloads) {
  const counts = opCounts(workload.target)
  console.log(
    `| ${workload.name} | ${number(counts.createElement ?? 0)} |` +
      ` ${number(counts.appendChild ?? 0)} | ${number(counts.setStyle ?? 0)} |` +
      ` ${number(counts.setText ?? 0)} | ${number(counts.setEventListener ?? 0)} |` +
      ` ${number(counts.setCustomProp ?? 0)} |` +
      ` ${milliseconds(workload.reactRenderMs)} | ${milliseconds(workload.reactCommitMs)} |`,
  )
}

console.log('\nNotes:')
for (const workload of workloads) console.log(`- ${workload.name}: ${workload.description}`)
console.log(
  '- Image rows measure mutation transport only. <img> sends a filesystem path; image decode, animation, GPU upload, layout, and paint are not timed.',
)
console.log(
  '- Raw <svg source> text does cross the boundary as a custom prop, so its bytes are included.',
)
console.log(
  '- React Profiler actualDuration includes component execution and Fiber render/reconciliation. Commit wall also includes host mutation batching and JSON.stringify, but fixture JSON.parse now runs outside the timed region.',
)

process.exit(0)
