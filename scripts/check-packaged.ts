import { chmod, copyFile, mkdtemp, rm } from "node:fs/promises"
import { basename, join, resolve } from "node:path"
import { tmpdir } from "node:os"

const source = process.argv[2]
if (!source) throw new Error("Usage: bun scripts/check-packaged.ts EXECUTABLE")
const directory = await mkdtemp(join(tmpdir(), "gpuix-isolated-check-"))
try {
  const executable = join(directory, basename(source))
  await copyFile(resolve(source), executable)
  await chmod(executable, 0o755)
  const child = Bun.spawn([executable, "--check"], {
    cwd: directory,
    stdin: "ignore", stdout: "inherit", stderr: "inherit",
    env: { ...process.env, LUA_PATH: undefined, LUA_CPATH: undefined, NODE_PATH: undefined },
  })
  const timeout = setTimeout(() => child.kill(), 60_000)
  try {
    const status = await child.exited
    if (status !== 0) throw new Error(`Isolated packaged application check failed: ${status}`)
  } finally {
    clearTimeout(timeout)
  }
  console.log("Packaged application starts without source sidecars")
} finally {
  await rm(directory, { recursive: true, force: true })
}
