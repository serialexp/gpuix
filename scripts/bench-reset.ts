import { spawn } from "node:child_process"
import { mkdir, writeFile } from "node:fs/promises"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const binary = process.env.GPX ?? resolve(root, "packages/native/target/release/gpx")
const entries = process.argv.slice(2)
const launches = Number(process.env.LAUNCHES ?? 5)
const iterations = Number(process.env.ITERATIONS ?? 20)
if (!entries.length || [launches, iterations].some(value => !Number.isInteger(value) || value < 1 || value > 1000)) {
  throw new Error("Usage: [LAUNCHES=5 ITERATIONS=20 GPX=path] bun scripts/bench-reset.ts APP ...")
}

interface Sample {
  loadMs: number
  mountMs: number
  disposeMs: number
  replaceViewMs: number
  drawMs: number
  totalMs: number
  nodes: number
}
interface Run {
  initial: Omit<Sample, "disposeMs" | "replaceViewMs"> & { windowMs: number; nativeInitAndOtherMs: number }
  resets: Sample[]
  scope: string
  windowId: number
  windowCount: number
  javascriptLoadIncludesMount: boolean
  processToSceneMs: number
}

async function run(entry: string): Promise<Run> {
  const start = performance.now()
  const child = spawn(binary, ["bench-reset", resolve(entry), "--iterations", String(iterations)], {
    cwd: root, stdio: ["ignore", "pipe", "pipe"],
  })
  let stdout = ""
  let stderr = ""
  let ready: number | undefined
  child.stdout.setEncoding("utf8").on("data", chunk => { stdout += chunk })
  child.stderr.setEncoding("utf8").on("data", chunk => {
    stderr += chunk
    if (ready === undefined && stderr.includes("gpx: reset-bench initial scene ready")) ready = performance.now() - start
  })
  const timer = setTimeout(() => child.kill("SIGKILL"), 120_000)
  try {
    await new Promise<void>((accept, reject) => {
      child.on("error", reject)
      child.on("close", code => code === 0 ? accept() : reject(new Error(`Benchmark exited ${code}: ${stderr}`)))
    })
  } finally { clearTimeout(timer) }
  if (ready === undefined) throw new Error("Missing initial scene marker")
  const result = JSON.parse(stdout) as Run
  if (result.windowCount !== 1 || result.resets.some(sample => sample.nodes !== result.initial.nodes)) {
    throw new Error("Benchmark must retain one native window and a consistent initial tree size")
  }
  return { ...result, processToSceneMs: ready }
}

function stats(values: number[]) {
  const sorted = [...values].sort((left, right) => left - right)
  const at = (fraction: number) => sorted[Math.max(0, Math.ceil(sorted.length * fraction) - 1)]
  return { medianMs: at(0.5), p95Ms: at(0.95), minMs: sorted[0], maxMs: sorted.at(-1)! }
}

const reports: { summary: Record<string, unknown>; runs: Run[] }[] = []
for (const entry of entries) {
  const runs: Run[] = []
  for (let index = 0; index < launches; index++) runs.push(await run(entry))
  const samples = runs.flatMap(run => run.resets)
  const phases = Object.fromEntries((["loadMs", "mountMs", "disposeMs", "replaceViewMs", "drawMs", "totalMs"] as const)
    .map(phase => [phase, stats(samples.map(sample => sample[phase]))]))
  const summary = {
    app: entry, launches, iterations, nodes: runs[0].initial.nodes,
    processToScene: stats(runs.map(run => run.processToSceneMs)),
    initial: Object.fromEntries((["loadMs", "mountMs", "windowMs", "drawMs", "nativeInitAndOtherMs", "totalMs"] as const)
      .map(phase => [phase, stats(runs.map(run => run.initial[phase]))])),
    reset: phases, javascriptLoadIncludesMount: runs[0].javascriptLoadIncludesMount,
  }
  reports.push({ summary, runs })
  console.log(JSON.stringify(summary, null, 2))
}
const output = resolve(root, "packages/react/screenshots/reset-bench/results.json")
await mkdir(dirname(output), { recursive: true })
await writeFile(output, JSON.stringify(reports, null, 2) + "\n")
console.log(`Raw samples: ${output}`)
