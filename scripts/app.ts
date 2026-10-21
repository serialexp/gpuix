import { readFile } from "node:fs/promises"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { loadManifest } from "./app-manifest"

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "..")

async function run(command: string[], env: Record<string, string> = {}) {
  const child = Bun.spawn(command, { cwd: repository, stdin: "inherit", stdout: "inherit", stderr: "inherit", env: { ...process.env, ...env } })
  const status = await child.exited
  if (status !== 0) throw new Error(`${command[0]} exited with status ${status}`)
}

async function main() {
  const [command, source, ...args] = process.argv.slice(2)
  if (command !== "dev" || !source) throw new Error("Usage: bun scripts/app.ts dev gpuix.json [-- launcher-options]\nFor packaging, use gpx package gpuix.json instead.")
  const { manifest, manifestPath, root } = await loadManifest(source)
  await run(["bun", "run", "--cwd", "packages/native", "build:embedded"])
  await run(["cargo", "build", "--release", "--manifest-path", "packages/native/Cargo.toml", "--no-default-features", "--features", "lua54,quickjs", "--bin", "gpx"], process.platform === "darwin" ? { MACOSX_DEPLOYMENT_TARGET: manifest.macos?.minimumSystemVersion ?? "14.0" } : {})
  let executable = join(repository, "packages/native/target/release", process.platform === "win32" ? "gpx.exe" : "gpx")
  if (process.platform === "darwin") {
    const icon = manifest.icon ? resolve(root, manifest.icon) : join(repository, "scripts/assets/AppIcon.icns")
    await readFile(icon)
    const app = join(repository, "dist/dev", `${manifest.name}.app`)
    await run(["bash", "scripts/macos-app.sh", executable, app, manifest.name,
      `${manifest.identifier}.development`, icon, manifest.version,
      manifest.macos?.minimumSystemVersion ?? "14.0",
      manifest.macos?.category ?? "public.app-category.developer-tools"])
    await run(["codesign", "--force", "--sign", "-", app])
    executable = join(app, "Contents/MacOS", manifest.name)
  }
  await run([executable, "--manifest", manifestPath, ...(args[0] === "--" ? args.slice(1) : args)])
}

main().catch(error => { console.error(`gpuix: ${error.message}`); process.exitCode = 1 })
