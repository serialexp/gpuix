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
import { spawnSync } from 'node:child_process'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { fileURLToPath } from 'node:url'
import { join } from 'node:path'
import { createRoot, flushSync } from '@gpuix/react'
import type { NativeRenderer } from '@gpuix/react'
import { ChatApp } from './chat'

type Op = unknown[]

interface NativeBatchRenderer {
  applyBatch(json: string): number[]
  applySnapshot(json: string): number[]
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

interface CapturedSnapshot {
  setup: string[]
  target: string
  reactRenderMs: number
  reactCommitMs: number
}

interface CapturedWorkload extends MutationWorkload {
  snapshot: CapturedSnapshot
}

interface WorkloadSpec {
  name: string
  description: string
  initial: React.ReactNode
  update?: React.ReactNode
}

class CaptureRenderer implements NativeRenderer {
  batches: string[] = []
  snapshots: string[] = []

  applyBatch(json: string): number[] {
    this.batches.push(json)
    return []
  }

  applySnapshot(json: string): number[] {
    this.snapshots.push(json)
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

function captureSnapshot(spec: WorkloadSpec): CapturedSnapshot {
  const renderer = new CaptureRenderer()
  const root = createRoot(renderer, { transport: 'snapshot' })
  let reactRenderMs = 0
  const onRender: React.ProfilerOnRenderCallback = (
    _id,
    _phase,
    actualDuration,
  ) => {
    reactRenderMs = actualDuration
  }
  const profiled = (node: React.ReactNode) => (
    <React.Profiler id={`${spec.name}-snapshot`} onRender={onRender}>
      {node}
    </React.Profiler>
  )

  let start = performance.now()
  flushSync(() => root.render(profiled(spec.initial)))
  const mountCommitMs = performance.now() - start
  const setup = [...renderer.snapshots]
  let reactCommitMs = mountCommitMs
  if (spec.update !== undefined) {
    start = performance.now()
    flushSync(() => root.render(profiled(spec.update)))
    reactCommitMs = performance.now() - start
  }
  const target = renderer.snapshots[renderer.snapshots.length - 1]!
  root.unmount()

  return {
    setup: spec.update === undefined ? [] : setup,
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

interface SnapshotNode {
  id: number
  type: string
  parent: number | null
  children: number[]
  style: unknown
  text: string | null
  events: Set<string>
  customProps: Map<string, unknown>
}

class SnapshotTree {
  nodes = new Map<number, SnapshotNode>()
  root: number | null = null

  apply(ops: Op[]): void {
    for (const op of ops) {
      const name = String(op[0])
      if (name === 'createElement') {
        const id = Number(op[1])
        this.nodes.set(id, {
          id,
          type: String(op[2]),
          parent: null,
          children: [],
          style: null,
          text: null,
          events: new Set(),
          customProps: new Map(),
        })
      } else if (name === 'destroyElement') {
        this.destroy(Number(op[1]))
      } else if (name === 'appendChild') {
        this.place(Number(op[1]), Number(op[2]), null)
      } else if (name === 'insertBefore') {
        this.place(Number(op[1]), Number(op[2]), Number(op[3]))
      } else if (name === 'setStyle') {
        const node = this.nodes.get(Number(op[1]))
        if (node) node.style = op[2]
      } else if (name === 'setText') {
        const node = this.nodes.get(Number(op[1]))
        if (node) node.text = String(op[2])
      } else if (name === 'setEventListener') {
        const node = this.nodes.get(Number(op[1]))
        if (!node) continue
        if (op[3]) node.events.add(String(op[2]))
        else node.events.delete(String(op[2]))
      } else if (name === 'setRoot') {
        this.root = Number(op[1])
      } else if (name === 'setCustomProp') {
        const node = this.nodes.get(Number(op[1]))
        if (!node) continue
        if (op[3] === null || op[3] === undefined) node.customProps.delete(String(op[2]))
        else node.customProps.set(String(op[2]), op[3])
      }
    }
  }

  private place(parentId: number, childId: number, beforeId: number | null): void {
    const parent = this.nodes.get(parentId)
    const child = this.nodes.get(childId)
    if (!parent || !child) return
    if (child.parent !== null) {
      const oldParent = this.nodes.get(child.parent)
      if (oldParent) oldParent.children = oldParent.children.filter((id) => id !== childId)
    }
    child.parent = parentId
    const before = beforeId === null ? -1 : parent.children.indexOf(beforeId)
    if (before === -1) parent.children.push(childId)
    else parent.children.splice(before, 0, childId)
  }

  private destroy(id: number): void {
    const node = this.nodes.get(id)
    if (!node) return
    if (node.parent !== null) {
      const parent = this.nodes.get(node.parent)
      if (parent) parent.children = parent.children.filter((child) => child !== id)
    }
    for (const child of node.children) this.destroy(child)
    this.nodes.delete(id)
    if (this.root === id) this.root = null
  }
}

function hashString(value: string, cache: Map<string, number>): number {
  const cached = cache.get(value)
  if (cached !== undefined) return cached
  let hash = 0x811c9dc5
  for (let index = 0; index < value.length; index++) {
    hash ^= value.charCodeAt(index)
    hash = Math.imul(hash, 0x01000193)
  }
  const result = hash >>> 0
  cache.set(value, result)
  return result
}

function hashValue(value: unknown, cache: Map<string, number>): number {
  if (value === null || value === undefined) return 0
  return hashString(typeof value === 'string' ? value : JSON.stringify(value), cache)
}

function encodeSnapshot(tree: SnapshotTree): Buffer {
  const recordBytes = 40
  const nodes = [...tree.nodes.values()].sort((left, right) => left.id - right.id)
  const positions = new Map<number, number>()
  for (const parent of nodes) {
    for (let index = 0; index < parent.children.length; index++) {
      positions.set(parent.children[index]!, index)
    }
  }
  const cache = new Map<string, number>()
  const output = Buffer.allocUnsafe(nodes.length * recordBytes)
  for (let index = 0; index < nodes.length; index++) {
    const node = nodes[index]!
    const customProps = Object.fromEntries(
      [...node.customProps.entries()].sort(([left], [right]) => left.localeCompare(right)),
    )
    const fields = [
      node.id,
      node.parent ?? 0xffffffff,
      positions.get(node.id) ?? 0,
      node.children.length,
      hashString(node.type, cache),
      hashValue(node.style, cache),
      hashValue(node.text, cache),
      hashString([...node.events].sort().join('\u0000'), cache),
      hashValue(customProps, cache),
      tree.root === node.id ? 1 : 0,
    ]
    for (let field = 0; field < fields.length; field++) {
      output.writeUInt32LE(fields[field]! >>> 0, index * recordBytes + field * 4)
    }
  }
  return output
}

interface SnapshotDiffResult {
  workload: string
  beforeNodes: number
  afterNodes: number
  encodeMs: number
  rustDiffMs: number
  changedNodes: number
}

function compileDiffPrototype(directory: string): string {
  const source = fileURLToPath(new URL('./bench-tree-diff.rs', import.meta.url))
  const binary = join(directory, process.platform === 'win32' ? 'bench-tree-diff.exe' : 'bench-tree-diff')
  const compiled = spawnSync('rustc', [source, '--edition=2021', '-O', '-o', binary], {
    encoding: 'utf8',
  })
  if (compiled.status !== 0) {
    throw new Error(`rustc failed:\n${compiled.stdout}\n${compiled.stderr}`)
  }
  return binary
}

function measureSnapshotDiff(
  workload: CapturedWorkload,
  binary: string,
  directory: string,
  iterations: number,
): SnapshotDiffResult {
  const tree = new SnapshotTree()
  for (const setup of workload.setup) tree.apply(setup)
  const encodeBefore = encodeSnapshot(tree)
  tree.apply(workload.target)
  const encodeStart = performance.now()
  const encodeAfter = encodeSnapshot(tree)
  const encodeMs = performance.now() - encodeStart
  const safeName = workload.name.replace(/ /g, '-')
  const beforePath = join(directory, `${safeName}-before.bin`)
  const afterPath = join(directory, `${safeName}-after.bin`)
  writeFileSync(beforePath, encodeBefore)
  writeFileSync(afterPath, encodeAfter)
  const run = spawnSync(binary, [beforePath, afterPath, String(iterations)], {
    encoding: 'utf8',
  })
  if (run.status !== 0) {
    throw new Error(`tree diff failed for ${workload.name}:\n${run.stdout}\n${run.stderr}`)
  }
  const result = JSON.parse(run.stdout) as {
    medianMs: number
    changed: number
    before: number
    after: number
  }
  return {
    workload: workload.name,
    beforeNodes: result.before,
    afterNodes: result.after,
    encodeMs,
    rustDiffMs: result.medianMs,
    changedNodes: result.changed,
  }
}

function rootId(workload: CapturedWorkload): number | null {
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

function measureSnapshotNative(
  renderer: NativeBatchRenderer,
  workload: CapturedWorkload,
  iterations: number,
): Timing {
  const payload = workload.snapshot.target
  const snapshot = JSON.parse(payload)
  const empty = '{"rootId":null,"nodes":[]}'
  const before = workload.snapshot.setup[workload.snapshot.setup.length - 1] ?? empty
  const reset = (): void => {
    renderer.applySnapshot(before)
  }

  for (let iteration = 0; iteration < 3; iteration++) {
    reset()
    renderer.applySnapshot(payload)
  }

  const stringify: number[] = []
  const native: number[] = []
  const endToEnd: number[] = []
  for (let iteration = 0; iteration < iterations; iteration++) {
    let start = performance.now()
    JSON.stringify(snapshot)
    stringify.push(performance.now() - start)

    reset()
    start = performance.now()
    renderer.applySnapshot(payload)
    native.push(performance.now() - start)

    reset()
    start = performance.now()
    renderer.applySnapshot(JSON.stringify(snapshot))
    endToEnd.push(performance.now() - start)
  }

  renderer.applySnapshot(empty)
  if (renderer.getRetainedElementCount() !== 0) {
    throw new Error(`${workload.name} snapshot left native elements behind after cleanup`)
  }

  return {
    stringifyMs: median(stringify),
    nativeMs: median(native),
    endToEndMs: median(endToEnd),
  }
}

interface CommitTiming {
  reactRenderMs: number
  wallMs: number
}

function measureNativeCommit(
  renderer: NativeBatchRenderer,
  spec: WorkloadSpec,
  transport: 'mutations' | 'snapshot',
  iterations: number,
): CommitTiming {
  const renders: number[] = []
  const walls: number[] = []
  for (let iteration = 0; iteration < iterations; iteration++) {
    const root = createRoot(renderer as NativeRenderer, { transport })
    let reactRenderMs = 0
    const onRender: React.ProfilerOnRenderCallback = (
      _id,
      _phase,
      actualDuration,
    ) => {
      reactRenderMs = actualDuration
    }
    const profiled = (node: React.ReactNode) => (
      <React.Profiler id={`${spec.name}-${transport}`} onRender={onRender}>
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
      throw new Error(`${spec.name} ${transport} commit left native elements behind`)
    }
  }
  return { reactRenderMs: median(renders), wallMs: median(walls) }
}

function measureNative(
  renderer: NativeBatchRenderer,
  workload: CapturedWorkload,
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

const workloads: CapturedWorkload[] = workloadSpecs.map((spec) => ({
  ...capture(spec.name, spec.description, spec.initial, spec.update),
  snapshot: captureSnapshot(spec),
}))

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

console.log('\n## Full snapshot transport with Rust reconciliation\n')
console.log('| workload | kind | nodes | JSON | stringify | napi + Rust | combined |')
console.log('|---|---|---:|---:|---:|---:|---:|')
for (const workload of workloads) {
  const timing = measureSnapshotNative(renderer, workload, iterations)
  const snapshot = JSON.parse(workload.snapshot.target) as { nodes: unknown[] }
  const bytes = Buffer.byteLength(workload.snapshot.target)
  console.log(
    `| ${workload.name} | ${workload.kind} | ${number(snapshot.nodes.length)} |` +
      ` ${(bytes / 1e6).toFixed(2)} MB | ${milliseconds(timing.stringifyMs)} |` +
      ` ${milliseconds(timing.nativeMs)} | **${milliseconds(timing.endToEndMs)}** |`,
  )
}

console.log('\n## Actual React commit through NAPI\n')
console.log('| workload | mutation render | mutation wall | snapshot render | snapshot wall |')
console.log('|---|---:|---:|---:|---:|')
for (const spec of workloadSpecs) {
  const mutation = measureNativeCommit(renderer, spec, 'mutations', commitIterations)
  const snapshot = measureNativeCommit(renderer, spec, 'snapshot', commitIterations)
  console.log(
    `| ${spec.name} | ${milliseconds(mutation.reactRenderMs)} |` +
      ` **${milliseconds(mutation.wallMs)}** | ${milliseconds(snapshot.reactRenderMs)} |` +
      ` **${milliseconds(snapshot.wallMs)}** |`,
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
console.log(
  '- Snapshot commit wall includes maintaining the JavaScript host snapshot, serializing the full snapshot, the NAPI crossing, Rust decode/reconciliation, and return.',
)

const diffDirectory = mkdtempSync(join(tmpdir(), 'gpuix-tree-diff-'))
try {
  const diffBinary = compileDiffPrototype(diffDirectory)
  const diffIterations = Number(process.env.DIFF_ITERATIONS ?? 500)
  const diffs = workloads.map((workload) =>
    measureSnapshotDiff(workload, diffBinary, diffDirectory, diffIterations),
  )
  console.log('\n## Compact host snapshot prototype\n')
  console.log('| workload | before | after | changed | JS snapshot encode | Rust diff |')
  console.log('|---|---:|---:|---:|---:|---:|')
  for (const result of diffs) {
    console.log(
      `| ${result.workload} | ${number(result.beforeNodes)} | ${number(result.afterNodes)} |` +
        ` ${number(result.changedNodes)} | ${milliseconds(result.encodeMs)} |` +
        ` ${milliseconds(result.rustDiffMs)} |`,
    )
  }
  console.log(
    '\nThe Rust prototype compares sorted 40-byte host records by stable id. It measures only a linear fixed-record diff: no React semantics, napi transfer, allocation of a new retained tree, or GPUI invalidation.',
  )
  console.log(
    'Snapshot encode starts after the captured mutations have already been applied to the JavaScript SnapshotTree, so producing that next tree is also outside the prototype timer.',
  )
} finally {
  rmSync(diffDirectory, { recursive: true, force: true })
}

process.exit(0)
