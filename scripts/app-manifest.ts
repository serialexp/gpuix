import { readFile, realpath } from "node:fs/promises"
import { dirname, isAbsolute } from "node:path"

export interface AppManifest {
  schemaVersion: 1
  name: string
  identifier: string
  version: string
  entry: string
  files: string[]
  icon?: string
  macos?: { minimumSystemVersion?: string; category?: string }
  linux?: { packageName?: string; maintainer?: string; description?: string; icon?: string; categories?: string[] }
}

export function appPath(value: unknown): asserts value is string {
  if (typeof value !== "string" || !value || isAbsolute(value) || /[\\:\0]/.test(value)
    || value.split("/").some(part => !part || part === "." || part === "..")) {
    throw new Error(`Expected an application-relative path, got ${JSON.stringify(value)}`)
  }
}

export function validateManifest(value: unknown): AppManifest {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Expected an application manifest object")
  const manifest = value as Record<string, unknown>
  const allowed = new Set(["schemaVersion", "name", "identifier", "version", "entry", "files", "icon", "macos", "linux"])
  for (const key of Object.keys(manifest)) if (!allowed.has(key)) throw new Error(`Unknown manifest field: ${key}`)
  if (manifest.schemaVersion !== 1) throw new Error("Expected schemaVersion: 1")
  if (typeof manifest.name !== "string" || !manifest.name.trim() || /[/\\\0]/.test(manifest.name)) throw new Error("Invalid application name")
  if (typeof manifest.identifier !== "string" || !/^[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+$/.test(manifest.identifier)) throw new Error("Expected a reverse-domain identifier")
  if (typeof manifest.version !== "string" || !/^\d+\.\d+\.\d+$/.test(manifest.version)) throw new Error("Expected a numeric major.minor.patch version")
  appPath(manifest.entry)
  if (!/\.(lua|luax|js|jsx|ts|tsx|mjs|mts)$/i.test(manifest.entry)) throw new Error("Unsupported application entry extension")
  if (!Array.isArray(manifest.files) || !manifest.files.length) throw new Error("Expected a nonempty files list")
  for (const file of manifest.files) if (file !== ".") appPath(file)
  if (manifest.icon !== undefined && (typeof manifest.icon !== "string" || !manifest.icon || !manifest.icon.endsWith(".icns"))) throw new Error("icon must name a macOS .icns file")
  if (manifest.macos !== undefined) {
    if (!manifest.macos || typeof manifest.macos !== "object" || Array.isArray(manifest.macos)) throw new Error("Expected macos options")
    for (const [key, setting] of Object.entries(manifest.macos)) {
      if (!["minimumSystemVersion", "category"].includes(key) || typeof setting !== "string" || !setting) throw new Error(`Invalid macos option: ${key}`)
    }
    const minimum = (manifest.macos as AppManifest["macos"])?.minimumSystemVersion
    if (minimum && !/^\d+(?:\.\d+){0,2}$/.test(minimum)) throw new Error("Invalid macOS minimumSystemVersion")
    if (minimum && Number(minimum.split(".")[0]) < 14) throw new Error("GPUIX currently requires macOS 14 or later")
  }
  if (manifest.linux !== undefined) {
    if (!manifest.linux || typeof manifest.linux !== "object" || Array.isArray(manifest.linux)) throw new Error("Expected linux options")
    for (const [key, setting] of Object.entries(manifest.linux)) {
      if (!["packageName", "maintainer", "description", "icon", "categories"].includes(key)) throw new Error(`Unknown linux option: ${key}`)
      if (key === "categories") {
        if (!Array.isArray(setting) || !setting.length || setting.some(item => typeof item !== "string" || !/^[A-Za-z][A-Za-z0-9-]*$/.test(item))) throw new Error("Invalid Linux desktop categories")
      } else if (typeof setting !== "string" || !setting.trim() || /[\r\n\0]/.test(setting)) throw new Error(`Invalid linux option: ${key}`)
    }
    const linux = manifest.linux as AppManifest["linux"]
    if (linux?.packageName && !/^[a-z0-9][a-z0-9+.-]+$/.test(linux.packageName)) throw new Error("Invalid Debian packageName")
    if (linux?.icon && !linux.icon.endsWith(".svg")) throw new Error("linux.icon must name an SVG file")
    if (linux?.maintainer && !/^[^<>]+ <[^<>\s]+@[^<>\s]+>$/.test(linux.maintainer)) throw new Error("Expected linux.maintainer as Name <email>")
  }
  return manifest as unknown as AppManifest
}

export async function loadManifest(path: string) {
  const manifestPath = await realpath(path)
  const manifest = validateManifest(JSON.parse(await readFile(manifestPath, "utf8")))
  return { manifest, manifestPath, root: dirname(manifestPath) }
}
