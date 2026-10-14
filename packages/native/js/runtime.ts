import { GpuixRenderer } from "@gpuix/native"
import type { EventPayload } from "@gpuix/native"
import {
  App,
  browserRendererAsTest,
  InProcessBackend,
  liveRendererAsTest,
  serveAutomationStdio,
  type LiveAutomationRenderer,
} from "./automation/client.js"
import { createRendererState } from "./renderer-state.js"

const DEFAULT_FRAME_MS = 8

export interface FrameLoop {
  stop(): void
}

export interface CreateNativeRendererOptions {
  onEvent?: (event: EventPayload) => void
  onError?: (error: unknown) => void
}

export function createNativeRenderer(
  options: CreateNativeRendererOptions = {}
): GpuixRenderer {
  let renderer!: GpuixRenderer
  // Returns whether a handler took the event. The napi addon ignores it;
  // the embedded runtime passes it back to the Rust host.
  renderer = new GpuixRenderer((error, event) => {
    if (error) {
      options.onError?.(error)
      return false
    }
    try {
      const handled = createRendererState(renderer).dispatch(event)
      if (handled) options.onEvent?.(event)
      return handled
    } catch (dispatchError) {
      options.onError?.(dispatchError)
      return false
    }
  })
  if (typeof process !== "undefined" && process.stdin && !process.stdin.isTTY) {
    const init = renderer.init.bind(renderer)
    renderer.init = (windowOptions) => {
      init(windowOptions)
      enableAutomation(renderer)
    }
  }
  return renderer
}

export function startFrameLoop(
  renderer: Pick<GpuixRenderer, "requiresTick" | "tick">,
  options: {
    frameMs?: number
    onTerminated?: () => void
    onError?: (error: unknown) => void
  } = {}
): FrameLoop {
  if (!renderer.requiresTick()) return { stop() {} }
  const frameMs = options.frameMs ?? DEFAULT_FRAME_MS
  let timer: ReturnType<typeof setTimeout> | undefined
  let stopped = false
  const stop = () => {
    stopped = true
    if (timer !== undefined) clearTimeout(timer)
    timer = undefined
  }
  const loop = () => {
    if (stopped) return
    const started = performance.now()
    let running = true
    try {
      running = renderer.tick()
    } catch (error) {
      options.onError?.(error)
    }
    if (running === false) {
      stop()
      options.onTerminated?.()
      return
    }
    timer = setTimeout(loop, Math.max(0, frameMs - (performance.now() - started)))
  }
  loop()
  return { stop }
}

export function enableAutomation(renderer: LiveAutomationRenderer): void {
  serveAutomationStdio(new InProcessBackend(liveRendererAsTest(renderer)))
}

export function installBrowserAutomation(
  renderer: LiveAutomationRenderer,
  existing?: App
): App {
  if (existing) return existing
  return new App(
    new InProcessBackend(browserRendererAsTest(renderer))
  )
}
