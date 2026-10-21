import { chmod, copyFile, mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises"
import { join, resolve } from "node:path"
import { tmpdir } from "node:os"

const source = process.argv[2]
if (!source) throw new Error("Usage: bun scripts/check-native-packager.ts GPX_EXECUTABLE")
const directory = await mkdtemp(join(tmpdir(), "gpuix-native-package-check-"))
const suffix = process.platform === "win32" ? ".exe" : ""
const tool = join(directory, `installed-gpx${suffix}`)
const env = {
  ...process.env,
  PATH: process.platform === "win32" ? `${process.env.SystemRoot}\\System32;${process.env.SystemRoot}` : "/usr/bin:/bin:/usr/sbin:/sbin",
  GPUIX_SOURCE_DIR: join(directory, "no-checkout"),
  LUA_PATH: undefined, LUA_CPATH: undefined, NODE_PATH: undefined,
}

async function run(executable: string, args: string[], success = true) {
  const child = Bun.spawn([executable, ...args], { cwd: directory, env, stdin: "ignore", stdout: "inherit", stderr: "inherit" })
  const timeout = setTimeout(() => child.kill(), 60_000)
  try {
    const status = await child.exited
    if ((status === 0) !== success) throw new Error(`Unexpected status ${status}: ${executable} ${args.join(" ")}`)
  } finally { clearTimeout(timeout) }
}

try {
  await copyFile(resolve(source), tool)
  await chmod(tool, 0o555)
  const app = join(directory, "source")
  await mkdir(join(app, "assets"), { recursive: true })
  await writeFile(join(app, "assets/value.txt"), "embedded asset")
  await writeFile(join(app, "assets/padding.bin"), Buffer.alloc(32768, 255))
  await writeFile(join(app, "message.lua"), 'return "Native packaging: "')
  await writeFile(join(app, "main.lua"), `local message = require("message")
local file = assert(io.open("assets/value.txt", "rb"))
local value = file.read(file, "*a")
file.close(file)
assert(value == "embedded asset")
local bytes = assert(io.open("assets/padding.bin", "rb"))
assert(bytes.read(bytes, 1) == string.char(255))
bytes.close(bytes)
return function() return gpuix.text(message .. value) end
`)
  const manifest = { schemaVersion: 1, name: "Native smoke", identifier: "dev.gpuix.native-smoke", version: "0.1.0", entry: "main.lua", files: ["main.lua", "message.lua", "assets"] }
  const path = join(app, "gpuix.json")
  await writeFile(path, JSON.stringify(manifest))
  const output = join(directory, `application${suffix}`)
  const protectedDirectory = join(directory, "existing-directory")
  await mkdir(protectedDirectory)
  await writeFile(join(protectedDirectory, "keep.txt"), "keep")
  await run(tool, ["package", path, "--format", "binary", "--output", protectedDirectory], false)
  if (await readFile(join(protectedDirectory, "keep.txt"), "utf8") !== "keep") throw new Error("Packaging replaced an existing directory")
  await run(tool, ["package", path, "--format", "binary", "--output", output])
  await run(output, ["--check"])
  const repacked = join(directory, `repacked${suffix}`)
  await run(output, ["package", path, "--format", "binary", "--output", repacked])
  if ((await stat(repacked)).size > (await stat(output)).size + 8192) throw new Error("Repackaging accumulated payloads")
  await run(repacked, ["--check"])
  const before = await readFile(output)
  await writeFile(join(app, "main.lua"), 'error("intentional packaging failure")')
  await run(tool, ["package", path, "--format", "binary", "--output", output], false)
  if (!before.equals(await readFile(output))) throw new Error("Failed packaging replaced an existing output")
  await writeFile(join(app, "message.js"), 'export default "Native JavaScript packaging"')
  await writeFile(join(app, "main.tsx"), 'import React from "react"; import { render } from "@gpuix/react"; import message from "./message.js"; render(<text>{message}</text>);')
  await writeFile(path, JSON.stringify({ ...manifest, entry: "main.tsx", files: ["main.tsx", "message.js"] }))
  await run(tool, ["package", path, "--format", "binary", "--output", join(directory, `javascript${suffix}`)])
  if (process.platform === "darwin") {
    const bundle = join(directory, "Native smoke.app")
    await run(tool, ["package", path, "--format", "app", "--output", bundle])
    await run(join(bundle, "Contents/MacOS/Native smoke"), ["--check"])
  }
  await rm(app, { recursive: true, force: true })
  await run(output, ["--check"])
  console.log("Native packaging works with only the installed runner, no source checkout or compiler/Bun PATH")
} finally { await rm(directory, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 }) }
