// Bundles the frameworks the embedded QuickJS runtime gives apps: React,
// `@gpuix/react`, Solid, `@gpuix/solid` and `@gpuix/native`, as ES modules
// in `embedded/dist/`.
//
//     bun run build:embedded      # from packages/native
//
// It writes two sets. `dist/production/` has the frameworks' production
// builds, which `gpx` runs apps with. `dist/development/` has their
// development builds and `react-refresh/runtime`, for `gpx --watch`: React
// Fast Refresh needs React's development build.
//
// In each set, one build with code splitting covers every module an app can
// import, so each package is in the output once. React and Solid each have a
// single copy, the one `@gpuix/react` or `@gpuix/solid` resolves. Each entry
// is named after its import specifier (`@gpuix/react.js`); shared code goes
// to `chunk-*.js`.
//
// `dist/manifest.txt` lists both sets' modules and every file the builds
// read, with a hash of them. `build.rs` checks the hash when the `quickjs`
// feature is on, so a stale bundle fails the build instead of being
// embedded.

import { existsSync, readFileSync, realpathSync, rmSync } from "node:fs"
import { createRequire } from "node:module"
import { dirname, join, relative, sep } from "node:path"
import type { BunPlugin } from "bun"

const NATIVE = dirname(import.meta.dir)
const REACT = join(NATIVE, "../react")
const SOLID = join(NATIVE, "../solid")
const DIST = join(NATIVE, "embedded/dist")
const MODES = ["production", "development"] as const
type Mode = (typeof MODES)[number]

// Paths in the manifest, and in Bun's metafile, are relative to the package,
// with forward slashes on every platform.
process.chdir(NATIVE)
const portable = (path: string) => path.split(sep).join("/")

// Package subpaths that need Node, Bun or the napi addon.
const SKIPPED_SUBPATHS = new Set(["./testing", "./automation", "./preload", "./bun-plugin"])

/** Every module an app can import, by specifier, with its entry source. */
type Entries = Map<string, string>

// React is CommonJS. An ES module can only export names it knows when it is
// built, so the entry lists them, read from `file`, one of its builds.
function commonJsEntry(entries: Entries, specifier: string, file: string): void {
  const resolved = Bun.resolveSync(specifier, REACT)
  const exports = createRequire(resolved)(join(dirname(resolved), file))
  const names = Object.keys(exports)
    .filter((name) => name !== "default" && /^[A-Za-z_$][\w$]*$/.test(name))
    .sort()
  entries.set(
    specifier,
    `import module from ${JSON.stringify(specifier)}\n` +
      `export const { ${names.join(", ")} } = module\n` +
      `export default module\n`
  )
}

// One scanner per syntax, picked by extension, so `<T>x` in a .ts file
// reads as a type assertion rather than JSX.
const scanners = new Map<string, Bun.Transpiler>()

function scanner(file: string): Bun.Transpiler {
  const extension = file.slice(file.lastIndexOf(".") + 1)
  const loader = extension === "tsx" || extension === "jsx" ? extension
    : extension === "ts" || extension === "mts" ? "ts"
    : "js"
  let scanner = scanners.get(loader)
  if (!scanner) {
    scanner = new Bun.Transpiler({ loader })
    scanners.set(loader, scanner)
  }
  return scanner
}

function reexportEntry(entries: Entries, specifier: string, file: string): void {
  const path = JSON.stringify(file)
  const hasDefault = scanner(file).scan(readFileSync(file, "utf8")).exports.includes("default")
  entries.set(
    specifier,
    `export * from ${path}\n${hasDefault ? `export { default } from ${path}\n` : ""}`
  )
}

/** The client build of an ES module package the package in `from` depends
 *  on, as its `exports` name it for browsers, in `mode`, and its manifest.
 *  Bun's own resolver would pick the `bun` or `node` condition, Solid's
 *  server build. */
function clientBuild(name: string, from: string, mode: Mode): { file: string; manifest: string } {
  let directory = from
  while (!existsSync(join(directory, "node_modules", name))) {
    if (dirname(directory) === directory) throw new Error(`Cannot find ${name} from ${from}`)
    directory = dirname(directory)
  }
  const packageDir = realpathSync(join(directory, "node_modules", name))
  const manifest = join(packageDir, "package.json")
  let target: unknown = JSON.parse(readFileSync(manifest, "utf8")).exports["."]
  while (typeof target === "object" && target !== null) {
    const conditions = target as Record<string, unknown>
    target = conditions.browser
      ?? (mode === "development" ? conditions.development : undefined)
      ?? conditions.default
  }
  if (typeof target !== "string") throw new Error(`No client build for ${name}`)
  return { file: join(packageDir, target), manifest }
}


/** A package's `dist/*.js` export, as the TypeScript source it is built from. */
function sourceOf(packageDir: string, sourceDir: string, target: string): string {
  if (!target.startsWith("./dist/")) return join(packageDir, target)
  const stem = join(packageDir, sourceDir, target.slice("./dist/".length, -".js".length))
  for (const extension of [".ts", ".tsx"]) {
    if (existsSync(stem + extension)) return stem + extension
  }
  throw new Error(`No source for ${target} in ${packageDir}`)
}

function packageEntries(entries: Entries, name: string, packageDir: string, sourceDir: string): void {
  const manifest = JSON.parse(readFileSync(join(packageDir, "package.json"), "utf8"))
  for (const [subpath, target] of Object.entries<{ import: string }>(manifest.exports)) {
    if (SKIPPED_SUBPATHS.has(subpath)) continue
    const specifier = subpath === "." ? name : `${name}/${subpath.slice(2)}`
    if (specifier === "@gpuix/native") {
      reexportEntry(entries, specifier, join(NATIVE, "embedded/native.ts"))
    } else {
      reexportEntry(entries, specifier, sourceOf(packageDir, sourceDir, target.import))
    }
  }
}

/** Fails the build on an import cycle among the modules it bundled, which
 *  code splitting may run in the wrong order (see `signals` below). */
function checkCycles(inputs: Record<string, { imports: { path: string }[] }>): void {
  const state = new Map<string, "open" | "done">()
  const stack: string[] = []
  const visit = (path: string): void => {
    state.set(path, "open")
    stack.push(path)
    for (const { path: next } of inputs[path]?.imports ?? []) {
      if (!(next in inputs)) continue
      if (state.get(next) === "open") {
        const cycle = [...stack.slice(stack.indexOf(next)), next]
        console.error(`An import cycle, which Bun's code splitting may run out of order:\n  ${cycle.join("\n  ")}`)
        process.exit(1)
      }
      if (!state.has(next)) visit(next)
    }
    stack.pop()
    state.set(path, "done")
  }
  for (const path of Object.keys(inputs)) if (!state.has(path)) visit(path)
}

/** What one set's build made and read. */
interface Built {
  mode: Mode
  entries: Entries
  /** The output files, relative to the set's directory. */
  files: string[]
  /** The files the build read, relative to the package. */
  inputs: string[]
  /** The package manifests it took client builds from. */
  manifests: string[]
  bytes: number
}

async function buildSet(mode: Mode): Promise<Built> {
  const out = join(DIST, mode)
  const entriesDir = join(DIST, `.entries-${mode}`)
  const define = { "process.env.NODE_ENV": JSON.stringify(mode) }
  const solidJs = clientBuild("solid-js", SOLID, mode)
  const solidUniversal = clientBuild("@solidjs/universal", SOLID, mode)
  const solidSignals = clientBuild("@solidjs/signals", dirname(solidJs.manifest), mode)

  const entries: Entries = new Map()
  commonJsEntry(entries, "react", `cjs/react.${mode}.js`)
  commonJsEntry(entries, "react/jsx-runtime", `cjs/react-jsx-runtime.${mode}.js`)
  if (mode === "development") {
    // Throws in its production build, so only `gpx --watch` has it.
    commonJsEntry(entries, "react-refresh/runtime", "cjs/react-refresh-runtime.development.js")
  }
  packageEntries(entries, "@gpuix/react", REACT, "src")
  reexportEntry(entries, "solid-js", solidJs.file)
  if (mode === "development") {
    const directory = dirname(solidJs.file)
    reexportEntry(entries, "solid-js/refresh", join(directory, "refresh.dev.js"))
    reexportEntry(entries, "solid-js/internal", join(directory, "internal.js"))
  }
  packageEntries(entries, "@gpuix/solid", SOLID, "src")
  packageEntries(entries, "@gpuix/native", NATIVE, "js")

  // Bun 1.4.0's code splitting can run the modules of an import cycle in the
  // wrong order once two entries share them: `@solidjs/signals`' core.js ran
  // before scheduler.js, which it imports and which imports it, and failed on
  // a class scheduler.js hadn't defined yet. Without splitting Bun keeps
  // their order, so signals is built alone first, into one module with no
  // cycle left, and the main build takes that. `checkCycles` fails the
  // build if another cycle turns up.
  const signalsFile = join(DIST, `.signals-${mode}/signals.js`)
  const signals = await Bun.build({
    entrypoints: [solidSignals.file],
    outdir: dirname(signalsFile),
    naming: "signals.js",
    target: "browser",
    format: "esm",
    define,
    metafile: true,
  })
  if (!signals.success) {
    for (const message of signals.logs) console.error(message)
    process.exit(1)
  }

  const plugin: BunPlugin = {
    name: "gpuix-embedded",
    setup(build) {
      // One React for everything: the copy `@gpuix/react` is built against.
      build.onResolve({ filter: /^(react|react-reconciler|react-refresh)(\/.*)?$/ }, ({ path }) => ({
        path: Bun.resolveSync(path, REACT),
      }))
      // One Solid: the client build `@gpuix/solid` is built against.
      build.onResolve({ filter: /^solid-js$/ }, () => ({ path: solidJs.file }))
      build.onResolve({ filter: /^@solidjs\/universal$/ }, () => ({ path: solidUniversal.file }))
      build.onResolve({ filter: /^@solidjs\/signals$/ }, () => ({ path: signalsFile }))
      build.onResolve({ filter: /^\.\/automation\/client\.js$|^@gpuix\/native\/automation$/ }, () => ({
        path: join(NATIVE, "embedded/automation.ts"),
      }))
      build.onResolve({ filter: /^@gpuix\/native$/ }, () => ({
        path: join(NATIVE, "embedded/native.ts"),
      }))
      // The TypeScript sources, so the bundle doesn't depend on `build:js`.
      build.onResolve({ filter: /^@gpuix\/native\/.+$/ }, ({ path }) => {
        const subpath = path.slice("@gpuix/native/".length)
        const file = join(NATIVE, "js", `${subpath}.ts`)
        return { path: existsSync(file) ? file : join(NATIVE, "js", subpath, "index.ts") }
      })
    },
  }

  const entryFiles: string[] = []
  for (const [specifier, source] of entries) {
    const file = join(entriesDir, `${specifier}.js`)
    await Bun.write(file, source)
    entryFiles.push(file)
  }

  const result = await Bun.build({
    entrypoints: entryFiles,
    root: entriesDir,
    outdir: out,
    target: "browser",
    format: "esm",
    splitting: true,
    naming: { entry: "[dir]/[name].[ext]", chunk: "chunk-[hash].[ext]" },
    define,
    metafile: true,
    plugins: [plugin],
  })
  if (!result.success) {
    for (const message of result.logs) console.error(message)
    process.exit(1)
  }
  checkCycles(result.metafile!.inputs)

  // The generated entries and signals module stand for the sources they
  // were made from.
  const generated = [entriesDir, dirname(signalsFile)].map((dir) => `${portable(relative(NATIVE, dir))}/`)
  const inputs = [...Object.keys(result.metafile!.inputs), ...Object.keys(signals.metafile!.inputs)]
    .map(portable)
    .filter((path) => !generated.some((dir) => path.startsWith(dir)))
  rmSync(entriesDir, { recursive: true, force: true })
  rmSync(dirname(signalsFile), { recursive: true, force: true })
  return {
    mode,
    entries,
    files: result.outputs.map((output) => portable(relative(out, output.path))).sort(),
    inputs,
    manifests: [solidJs, solidUniversal, solidSignals].map(({ manifest }) => portable(relative(NATIVE, manifest))),
    bytes: result.outputs.reduce((total, output) => total + output.size, 0),
  }
}

rmSync(DIST, { recursive: true, force: true })
const sets: Built[] = []
for (const mode of MODES) sets.push(await buildSet(mode))

// Besides the sources, the builds read the package manifests (exports) and
// the tsconfig files Bun takes JSX settings from.
const inputs = [
  ...new Set([
    ...sets.flatMap((set) => [...set.inputs, ...set.manifests]),
    "embedded/build.ts",
    "embedded/tsconfig.json",
    "package.json",
    "tsconfig.json",
    "../react/package.json",
    "../react/tsconfig.json",
    "../solid/package.json",
    "../solid/tsconfig.json",
  ]),
].sort()

// FNV-1a, 64-bit, over each input's path and bytes, each followed by a zero
// byte. `build.rs` computes the same.
let hash = 0xcbf29ce484222325n
const encoder = new TextEncoder()
for (const path of inputs) {
  const bytes = new Uint8Array(await Bun.file(path).arrayBuffer())
  for (const byte of [...encoder.encode(path), 0, ...bytes, 0]) {
    hash ^= BigInt(byte)
    hash = (hash * 0x100000001b3n) & 0xffffffffffffffffn
  }
}

const lines = [
  "# Written by embedded/build.ts. build.rs checks `hash` against the inputs.",
  `hash ${hash.toString(16).padStart(16, "0")}`,
  ...inputs.map((path) => `input ${path}`),
  ...sets.flatMap((set) => [
    ...[...set.entries.keys()].map((specifier) => `module ${set.mode} ${specifier} ${specifier}.js`),
    ...set.files.map((file) => `file ${set.mode} ${file}`),
  ]),
]
await Bun.write(join(DIST, "manifest.txt"), `${lines.join("\n")}\n`)

for (const set of sets) {
  console.log(`${set.mode}: ${set.entries.size} modules, ${set.files.length} files, ${(set.bytes / 1024).toFixed(0)} KiB`)
}
