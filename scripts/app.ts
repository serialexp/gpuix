import { chmod, copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { dirname, join, resolve, sep } from "node:path"
import { tmpdir } from "node:os"
import { fileURLToPath } from "node:url"
import { collectFiles, loadManifest } from "./app-manifest"
import { debMetadata, packageDeb } from "./deb"

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const outputRoot = process.env.GPUIX_PACKAGE_OUTPUT_ROOT ?? repository

async function run(command: string[], env: Record<string, string | undefined> = {}) {
  const child = Bun.spawn(command, {
    cwd: repository, stdin: "inherit", stdout: "inherit", stderr: "inherit",
    env: { ...process.env, GPUIX_APP_PAYLOAD: undefined, ...env },
  })
  const status = await child.exited
  if (status !== 0) throw new Error(`${command[0]} exited with status ${status}`)
}

async function main() {
  const [command, source, ...args] = process.argv.slice(2)
  if (!["dev", "package"].includes(command ?? "") || !source) {
    throw new Error("Usage: bun scripts/app.ts dev gpuix.json [-- launcher-options]\n       bun scripts/app.ts package gpuix.json [--format app|binary|deb] [--output PATH]")
  }
  const { manifest, manifestPath, root } = await loadManifest(source)
  const icon = manifest.icon ? resolve(root, manifest.icon) : join(repository, "scripts/assets/AppIcon.icns")
  if (process.platform === "darwin") await readFile(icon)
  let format = process.platform === "darwin" ? "app" : "binary"
  let output: string | undefined
  if (command === "package") {
    for (let index = 0; index < args.length; index += 2) {
      const value = args[index + 1]
      if (!value) throw new Error(`${args[index]} requires a value`)
      if (args[index] === "--format" && ["app", "binary", "deb"].includes(value)) format = value
      else if (args[index] === "--output") output = resolve(value)
      else throw new Error(`Unknown package option ${args[index]}`)
    }
    if (format === "app" && process.platform !== "darwin") throw new Error("App bundles currently require macOS")
    if (format === "deb") {
      if (process.platform !== "linux") throw new Error("Debian packages currently require Linux")
      debMetadata(manifest, process.arch === "arm64" ? "arm64" : "amd64", "pending")
      if (output && !output.endsWith(".deb")) throw new Error("Debian output must end with .deb")
    }
  }
  const stage = await mkdtemp(join(tmpdir(), "gpuix-package-"))
  try {
    let payload: string | undefined
    if (command === "package") {
      const files = await collectFiles(root, manifest.files)
      if (!files.some(file => file.path === manifest.entry)) throw new Error("The files list does not include the entry")
      payload = join(stage, "payload.json")
      await writeFile(payload, JSON.stringify({ formatVersion: 1, manifest, files }))
    }
    await run(["bun", "run", "--cwd", "packages/native", "build:embedded"])
    await run(["cargo", "build", "--release", "--manifest-path", "packages/native/Cargo.toml", "--no-default-features", "--features", "lua54,quickjs", "--bin", "gpx-app"], {
      GPUIX_APP_PAYLOAD: payload,
      ...(process.platform === "darwin" ? { MACOSX_DEPLOYMENT_TARGET: manifest.macos?.minimumSystemVersion ?? "14.0" } : {}),
    })
    const binary = join(repository, "packages/native/target/release", process.platform === "win32" ? "gpx-app.exe" : "gpx-app")
    let executable = binary
    if (format === "app") {
      const app = output ?? join(outputRoot, command === "dev" ? "dist/dev" : "dist", `${manifest.name}.app`)
      if (!app.endsWith(".app")) throw new Error("App bundle output must end with .app")
      if (root === app || root.startsWith(app + sep)) throw new Error("Output must not replace the application sources")
      const stagedApp = join(stage, `${manifest.name}.app`)
      await run(["bash", "scripts/macos-app.sh", binary, stagedApp, manifest.name,
        command === "dev" ? `${manifest.identifier}.development` : manifest.identifier,
        icon, manifest.version, manifest.macos?.minimumSystemVersion ?? "14.0",
        manifest.macos?.category ?? "public.app-category.developer-tools"])
      await run(["codesign", "--force", "--sign", "-", stagedApp])
      await run(["codesign", "--verify", "--strict", stagedApp])
      await rm(app, { recursive: true, force: true })
      await run(["ditto", stagedApp, app])
      executable = join(app, "Contents/MacOS", manifest.name)
    } else if (command === "package") {
      executable = format === "deb" ? join(stage, "gpx") : output ?? join(outputRoot, "dist", manifest.name + (process.platform === "win32" ? ".exe" : ""))
      if (resolve(executable) === resolve(binary)) throw new Error("Output must not overwrite the build runner")
      await mkdir(dirname(executable), { recursive: true })
      await copyFile(binary, executable)
      await chmod(executable, 0o755)
      if (process.platform === "darwin") await run(["codesign", "--force", "--sign", "-", executable])
    }
    if (command === "package") {
      await run([executable, "--check"])
      const artifact = format === "deb" ? await packageDeb(manifest, root, executable, repository, output, outputRoot) : output ?? (format === "app" ? dirname(dirname(dirname(executable))) : executable)
      console.log(`Packaged: ${artifact}`)
    } else {
      await run([executable, "--manifest", manifestPath, ...(args[0] === "--" ? args.slice(1) : args)])
    }
  } finally {
    await rm(stage, { recursive: true, force: true })
  }
}

main().catch(error => { console.error(`gpuix: ${error.message}`); process.exitCode = 1 })
