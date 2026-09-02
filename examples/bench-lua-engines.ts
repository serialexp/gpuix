import { spawnSync } from "node:child_process"
import { copyFileSync, existsSync, mkdirSync, readdirSync } from "node:fs"
import { fileURLToPath } from "node:url"
import path from "node:path"

const examplesDir = fileURLToPath(new URL(".", import.meta.url))
const nativeDir = path.join(examplesDir, "../packages/native")
const snapshotsDir = path.join(examplesDir, "../tmp/bench-lua")
const luajitPath = path.join(snapshotsDir, "gpuix-native-luajit.node")
const lua54Path = path.join(snapshotsDir, "gpuix-native-lua54.node")
const modes = [
  { label: "Full tree", env: { MEMO: "0", BATCH: "0", WARMUP: "20" } },
  { label: "Per-row memo", env: { MEMO: "1", BATCH: "0", WARMUP: "100" } },
  { label: "Memo batch", env: { MEMO: "0", BATCH: "1", WARMUP: "200" } },
]

function run(args: string[], cwd: string, env: NodeJS.ProcessEnv = {}) {
  const result = spawnSync("bun", args, {
    cwd,
    env: { ...process.env, ...env },
    stdio: "inherit",
  })
  if (result.status !== 0) {
    throw new Error(`bun ${args.join(" ")} failed with status ${result.status}`)
  }
}

function snapshot(engine: string) {
  const binaries = readdirSync(nativeDir).filter(
    (name) => name.startsWith("gpuix-native.") && name.endsWith(".node")
  )
  if (binaries.length !== 1) {
    throw new Error(`expected one local native binary, found ${binaries.length}`)
  }
  mkdirSync(snapshotsDir, { recursive: true })
  const destination = path.join(snapshotsDir, `gpuix-native-${engine}.node`)
  copyFileSync(path.join(nativeDir, binaries[0]), destination)
  return destination
}

function benchmark(engine: string, nativePath: string, mode: (typeof modes)[number]) {
  console.log(`\n--- ${engine}`)
  run(["bench-lua.ts"], examplesDir, {
    ENGINE: engine,
    NAPI_RS_NATIVE_LIBRARY_PATH: nativePath,
    ...mode.env,
    ...(process.env.WARMUP ? { WARMUP: process.env.WARMUP } : {}),
  })
}

let lua54Active = false
try {
  if (process.env.SKIP_BUILD === "1") {
    if (!existsSync(luajitPath) || !existsSync(lua54Path)) {
      throw new Error("cached engine snapshots are missing; run without SKIP_BUILD first")
    }
    lua54Active = true
  } else {
    console.log("\n=== Building LuaJIT ===")
    run(["run", "build:luajit"], nativeDir)
    snapshot("luajit")

    console.log("\n=== Building Lua 5.4 ===")
    run(["run", "build"], nativeDir)
    lua54Active = true
    snapshot("lua54")
  }

  const cooldownMs = Number(process.env.COOLDOWN_MS ?? 30_000)
  if (cooldownMs > 0) {
    console.log(`\nCooling down for ${(cooldownMs / 1000).toFixed(1)}s`)
    await Bun.sleep(cooldownMs)
  }

  const betweenMs = Number(process.env.BETWEEN_MS ?? 5_000)
  const engines = [
    { name: "LuaJIT", path: luajitPath },
    { name: "Lua 5.4", path: lua54Path },
  ]
  for (const [index, mode] of modes.entries()) {
    console.log(`\n=== ${mode.label} ===`)
    const orderedEngines = index % 2 === 0 ? engines : [...engines].reverse()
    for (const engine of orderedEngines) {
      benchmark(engine.name, engine.path, mode)
      if (betweenMs > 0) await Bun.sleep(betweenMs)
    }
  }
} finally {
  if (!lua54Active) {
    console.log("\n=== Restoring Lua 5.4 ===")
    run(["run", "build"], nativeDir)
  }
}
