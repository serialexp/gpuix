import { expect, test } from "vitest"
import { validateManifest } from "./app-manifest"
import { debMetadata } from "./deb"

const manifest = {
  schemaVersion: 1, name: "My App", identifier: "dev.example.app", version: "1.2.3",
  entry: "main.lua", files: ["main.lua"],
  linux: { maintainer: "Example Developer <dev@example.com>" },
}

test("generates Debian control metadata and a shell-free desktop launcher", () => {
  const metadata = debMetadata(validateManifest(manifest), "amd64", "libc6 (>= 2.39), libvulkan1")
  expect(metadata.name).toBe("dev.example.app")
  expect(metadata.control).toContain("Architecture: amd64\n")
  expect(metadata.control).toContain("Version: 1.2.3\n")
  expect(metadata.control).toContain("Depends: libc6 (>= 2.39), libvulkan1\n")
  expect(metadata.desktop).toContain("Exec=/usr/bin/dev.example.app\n")
  expect(metadata.desktop).toContain("Name=My App\n")
  expect(metadata.desktop).toContain("Categories=Utility;\n")
  expect(metadata.desktop).toContain("Terminal=false\n")
})

test("supports package naming, descriptions, categories and arm64", () => {
  const custom = validateManifest({ ...manifest, linux: {
    ...manifest.linux, packageName: "my-app", description: "An example", categories: ["Development", "Utility"], icon: "assets/app.svg",
  } })
  const metadata = debMetadata(custom, "arm64", "libc6")
  expect(metadata.name).toBe("my-app")
  expect(metadata.control).toContain("Architecture: arm64\n")
  expect(metadata.control).toContain("Description: An example\n")
  expect(metadata.desktop).toContain("Categories=Development;Utility;\n")
})

test("rejects missing maintainer, unsupported architectures and metadata injection", () => {
  expect(() => debMetadata(validateManifest({ ...manifest, linux: undefined }), "amd64", "libc6")).toThrow("maintainer")
  expect(() => debMetadata(validateManifest(manifest), "i386", "libc6")).toThrow("architecture")
  for (const change of [
    { packageName: "../app" }, { packageName: "Bad Package" },
    { maintainer: "dev\nDepends: other" }, { description: "Line\nInjected: value" },
    { categories: ["Utility;Exec=evil"] }, { icon: "app.icns" }, { unknown: "value" },
  ]) expect(() => validateManifest({ ...manifest, linux: { ...manifest.linux, ...change } })).toThrow()
})

test("escapes desktop display strings without treating them as executable arguments", () => {
  const custom = validateManifest({ ...manifest, name: 'App " $ % example', linux: { ...manifest.linux, description: 'Path \\ example' } })
  const metadata = debMetadata(custom, "amd64", "libc6")
  expect(metadata.desktop).toContain('Name=App " $ % example\n')
  expect(metadata.desktop).toContain('Comment=Path \\\\ example\n')
  expect(metadata.desktop).toContain("Exec=/usr/bin/dev.example.app\n")
})
