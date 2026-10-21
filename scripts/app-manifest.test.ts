import { expect, test } from "vitest"
import { validateManifest } from "./app-manifest"

const manifest = {
  schemaVersion: 1, name: "Test", identifier: "dev.gpuix.test",
  version: "0.1.0", entry: "main.lua", files: ["main.lua"],
}

test("validates application identity and rejects unimplemented permissions", () => {
  expect(validateManifest(manifest)).toEqual(manifest)
  for (const change of [
    { schemaVersion: 2 }, { entry: "../main.lua" }, { files: ["/etc"] },
    { permissions: {} }, { name: "../Test" }, { version: "next" },
    { macos: { minimumSystemVersion: "13.0" } },
  ]) expect(() => validateManifest({ ...manifest, ...change })).toThrow()
})

test("validates platform metadata for development bundles", () => {
  expect(validateManifest({ ...manifest, linux: { packageName: "my-app", maintainer: "Example <dev@example.com>", categories: ["Utility"] } }).linux?.packageName).toBe("my-app")
  for (const linux of [
    { packageName: "../app" }, { packageName: "Bad Package" },
    { maintainer: "dev\nDepends: other" }, { description: "Line\nInjected: value" },
    { categories: ["Utility;Exec=evil"] }, { icon: "app.icns" }, { unknown: "value" },
  ]) expect(() => validateManifest({ ...manifest, linux })).toThrow()
})
