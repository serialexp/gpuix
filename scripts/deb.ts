import { chmod, copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { dirname, join, resolve } from "node:path"
import { tmpdir } from "node:os"
import type { AppManifest } from "./app-manifest"

export function debMetadata(manifest: AppManifest, architecture: string, dependencies: string) {
  if (!["amd64", "arm64"].includes(architecture)) throw new Error(`Unsupported Debian architecture: ${architecture}`)
  const name = manifest.linux?.packageName ?? manifest.identifier.toLowerCase()
  const maintainer = manifest.linux?.maintainer
  if (!maintainer) throw new Error("Debian packaging requires linux.maintainer: Name <email>")
  if (/[\r\n\0]/.test(manifest.linux?.description ?? manifest.name)) throw new Error("Debian description must be a single line")
  const escape = (value: string) => value.replace(/\\/g, "\\\\").replace(/\n/g, "\\n").replace(/\r/g, "\\r").replace(/\t/g, "\\t")
  const control = [
    `Package: ${name}`, `Version: ${manifest.version}`, "Section: utils", "Priority: optional",
    `Architecture: ${architecture}`, `Maintainer: ${maintainer}`, `Depends: ${dependencies}`,
    `Description: ${manifest.linux?.description ?? manifest.name}`, "",
  ].join("\n")
  const desktop = [
    "[Desktop Entry]", "Type=Application", `Name=${escape(manifest.name)}`,
    `Comment=${escape(manifest.linux?.description ?? manifest.name)}`,
    `Exec=/usr/bin/${name}`, `TryExec=/usr/bin/${name}`, `Icon=${name}`,
    "Terminal=false", `Categories=${(manifest.linux?.categories ?? ["Utility"]).join(";")};`, "",
  ].join("\n")
  return { name, control, desktop }
}

async function command(args: string[], cwd: string) {
  const child = Bun.spawn(args, { cwd, stdout: "pipe", stderr: "inherit" })
  const output = await new Response(child.stdout).text()
  if (await child.exited !== 0) throw new Error(`${args[0]} failed`)
  return output.trim()
}

export async function packageDeb(manifest: AppManifest, root: string, binary: string, repository: string, output?: string, outputRoot = repository) {
  if (process.platform !== "linux") throw new Error("Debian packaging requires a native Debian/Ubuntu build host")
  const architecture = await command(["dpkg", "--print-architecture"], repository)
  const metadata = debMetadata(manifest, architecture, "pending")
  const stage = await mkdtemp(join(tmpdir(), "gpuix-deb-"))
  try {
    const tree = join(stage, "package")
    for (const path of ["DEBIAN", "usr/bin", "usr/share/applications", "usr/share/icons/hicolor/scalable/apps"]) await mkdir(join(tree, path), { recursive: true })
    const installed = join(tree, "usr/bin", metadata.name)
    const header = await readFile(binary)
    const machine = architecture === "amd64" ? 62 : 183
    if (header.length < 20 || header.subarray(0, 4).toString("hex") !== "7f454c46" || header[4] !== 2 || header[5] !== 1 || header.readUInt16LE(18) !== machine) throw new Error("Executable must be a native 64-bit Linux ELF matching the package architecture")
    await copyFile(binary, installed)
    await chmod(installed, 0o755)
    await copyFile(manifest.linux?.icon ? resolve(root, manifest.linux.icon) : join(repository, "scripts/assets/app-icon.svg"), join(tree, "usr/share/icons/hicolor/scalable/apps", `${metadata.name}.svg`))
    await writeFile(join(tree, "usr/share/applications", `${metadata.name}.desktop`), metadata.desktop)
    await command(["desktop-file-validate", join(tree, "usr/share/applications", `${metadata.name}.desktop`)], stage)
    await mkdir(join(stage, "debian"))
    await writeFile(join(tree, "DEBIAN/control"), metadata.control)
    await writeFile(join(stage, "debian/control"), `Source: ${metadata.name}\nSection: utils\nPriority: optional\nMaintainer: ${manifest.linux!.maintainer}\nStandards-Version: 4.6.2\n\n${metadata.control}`)
    const detected = await command(["dpkg-shlibdeps", "-O", `-e${installed}`], stage)
    const linked = detected.split("\n").find(line => line.startsWith("shlibs:Depends="))?.slice("shlibs:Depends=".length)
    if (!linked) throw new Error("dpkg-shlibdeps did not report runtime dependencies")
    const runtime = ["libvulkan1", "libwayland-client0", "libwayland-cursor0", "libwayland-egl1", "libxkbcommon0", "libxkbcommon-x11-0", "libx11-6", "libxcb1", "libfontconfig1", "libfreetype6"]
    const extra = runtime.filter(name => !linked.split(",").some(dependency => dependency.trim().split(/\s/)[0] === name))
    await writeFile(join(tree, "DEBIAN/control"), debMetadata(manifest, architecture, [linked, ...extra].join(", ")).control)
    const destination = output ?? join(outputRoot, "dist", `${metadata.name}_${manifest.version}_${architecture}.deb`)
    await mkdir(dirname(destination), { recursive: true })
    await command(["dpkg-deb", "--root-owner-group", "--build", tree, destination], stage)
    return destination
  } finally {
    await rm(stage, { recursive: true, force: true })
  }
}
