import { afterEach, expect, test } from "vitest"
import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { collectFiles, validateManifest } from "./app-manifest"

const temporary: string[] = []
const manifest = {
  schemaVersion: 1, name: "Test", identifier: "dev.gpuix.test",
  version: "0.1.0", entry: "main.lua", files: ["main.lua"],
}

afterEach(async () => {
  await Promise.all(temporary.splice(0).map(path => rm(path, { recursive: true, force: true })))
})

test("validates application identity and rejects unimplemented permissions", () => {
  expect(validateManifest(manifest)).toEqual(manifest)
  for (const change of [
    { schemaVersion: 2 }, { entry: "../main.lua" }, { files: ["/etc"] },
    { permissions: {} }, { name: "../Test" }, { version: "next" },
    { macos: { minimumSystemVersion: "13.0" } },
  ]) expect(() => validateManifest({ ...manifest, ...change })).toThrow()
})

test("collects nested binary assets deterministically and deduplicates includes", async () => {
  const root = await mkdtemp(join(tmpdir(), "gpuix-files-test-"))
  temporary.push(root)
  await mkdir(join(root, "assets"))
  await writeFile(join(root, "main.lua"), "return 1")
  await writeFile(join(root, "assets/raw.bin"), Buffer.from([0, 255]))
  const files = await collectFiles(root, [".", "main.lua"])
  expect(files.map(file => file.path)).toEqual(["assets/raw.bin", "main.lua"])
  expect(Buffer.from(files[0].data, "base64")).toEqual(Buffer.from([0, 255]))
})

test("rejects symlinks, including symlinks in parent directories", async () => {
  const root = await mkdtemp(join(tmpdir(), "gpuix-links-test-"))
  temporary.push(root)
  await mkdir(join(root, "real"))
  await writeFile(join(root, "real/main.lua"), "return 1")
  await symlink(join(root, "real"), join(root, "link"))
  await expect(collectFiles(root, ["link"])).rejects.toThrow("symlinks")
  await expect(collectFiles(root, ["link/main.lua"])).rejects.toThrow("symlinks")
  await expect(collectFiles(root, ["../escape"])).rejects.toThrow()
})
