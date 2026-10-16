import { transformAsync } from "@solidjs/compiler"
import { plugin as registerBunPlugin, type BunPlugin } from "bun"
import { readFile } from "node:fs/promises"
import { dirname, relative, resolve, sep } from "node:path"

const PLUGIN_STATE = Symbol.for("@gpuix/solid/bun-plugin")

/** The control flow `@gpuix/solid` re-exports, which compiled JSX imports
 *  from it. Solid's default list also names Portal and Dynamic, which are
 *  `@solidjs/web`'s. */
const BUILT_INS = ["For", "Show", "Switch", "Match", "Loading", "Reveal", "Repeat", "Errored"]

function stripQuery(path: string): string {
  return path.split(/[?#]/, 1)[0]!
}

/** Solid's development builds, with their checks and diagnostics, unless
 *  NODE_ENV is "production", as React picks its build. */
function development(): boolean {
  return process.env.NODE_ENV !== "production"
}

async function transformSource(code: string, filename: string, dev: boolean): Promise<string> {
  const result = await transformAsync(code, {
    filename,
    generate: "universal",
    moduleName: "@gpuix/solid",
    builtIns: BUILT_INS,
    dev,
  })
  return result.code
}

/** Load `target`, a file in the same package's `dist`, in place of `path`. */
async function loadInstead(path: string, target: string): Promise<{ contents: string; loader: "js" }> {
  const file = resolve(stripQuery(path))
  const dist = file.slice(0, file.lastIndexOf(`${sep}dist${sep}`) + `${sep}dist${sep}`.length)
  const replacement = resolve(dist, target)
  if (replacement === file) {
    return { contents: await readFile(file, "utf8"), loader: "js" }
  }
  const specifier = relative(dirname(file), replacement).split(sep).join("/")
  return { contents: `export * from ${JSON.stringify(`./${specifier}`)}\n`, loader: "js" }
}

const NODE_MODULES = String.raw`[/\\]node_modules[/\\]`
const QUERY = String.raw`(?:[?#].*)?$`

export function createSolidPlugin(): BunPlugin {
  return {
    name: "gpuix-solid",
    setup(build) {
      // Once, so a NODE_ENV change can't mix the two builds in one process.
      const dev = development()
      // Bun resolves solid-js's `node` condition, its server build, which
      // doesn't render, and doesn't resolve `development`. Each package loads
      // the client build NODE_ENV picks, so all of them agree.
      build.onLoad(
        { filter: new RegExp(String.raw`${NODE_MODULES}solid-js[/\\]dist[/\\](server|solid)(\.dev)?\.js${QUERY}`) },
        ({ path }) => loadInstead(path, dev ? "solid.dev.js" : "solid.js")
      )
      build.onLoad(
        { filter: new RegExp(String.raw`${NODE_MODULES}@solidjs[/\\]signals[/\\]dist[/\\](prod[/\\]index|dev)\.js${QUERY}`) },
        ({ path }) => loadInstead(path, dev ? "dev.js" : "prod/index.js")
      )
      build.onLoad(
        { filter: new RegExp(String.raw`${NODE_MODULES}@solidjs[/\\]signals[/\\]dist[/\\](prod[/\\]attribution|dev\.attribution)\.js${QUERY}`) },
        ({ path }) => loadInstead(path, dev ? "dev.attribution.js" : "prod/attribution.js")
      )
      build.onLoad(
        { filter: new RegExp(String.raw`${NODE_MODULES}@solidjs[/\\]universal[/\\]dist[/\\]universal(\.dev)?\.js${QUERY}`) },
        ({ path }) => loadInstead(path, dev ? "universal.dev.js" : "universal.js")
      )
      build.onLoad(
        { filter: /^(?!.*[/\\]node_modules[/\\]).*\.[cm]?[jt]sx(?:[?#].*)?$/ },
        async ({ path }) => ({
          contents: await transformSource(await readFile(stripQuery(path), "utf8"), stripQuery(path), dev),
          // The compiler keeps TypeScript syntax, for Bun to strip.
          loader: "ts",
        })
      )
    },
  }
}

export function ensureSolidPlugin(): boolean {
  const state = globalThis as typeof globalThis & { [PLUGIN_STATE]?: boolean }
  if (state[PLUGIN_STATE]) return false
  void registerBunPlugin(createSolidPlugin())
  state[PLUGIN_STATE] = true
  return true
}

const solidPlugin = createSolidPlugin()
export default solidPlugin
