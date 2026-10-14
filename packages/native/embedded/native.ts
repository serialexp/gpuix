// `@gpuix/native` in the embedded QuickJS runtime. There is no napi addon:
// the Rust host owns the window and drives the run loop, and installs
// `__gpuix` before the app runs. This renderer forwards batches to the
// window's tree and passes the host's events to the callback, as the addon
// does.
import type { EventPayload, WindowOptions } from "../index.js"

interface EmbeddedHost {
  renderer: { applyBatch(json: string): number[] }
  setEventHandler(handler: ((event: EventPayload) => unknown) | undefined): void
}

declare const __gpuix: EmbeddedHost

export class GpuixRenderer {
  constructor(eventCallback?: ((err: Error | null, event: EventPayload) => unknown) | null) {
    __gpuix.setEventHandler(eventCallback ? (event) => eventCallback(null, event) : undefined)
  }

  /** The host opens the window from its own command line; options aren't applied yet. */
  init(_options?: WindowOptions | null): void {}

  applyBatch(json: string): number[] {
    return __gpuix.renderer.applyBatch(json)
  }

  /** The host runs the loop, so `startFrameLoop` has nothing to drive. */
  requiresTick(): boolean {
    return false
  }

  tick(): boolean {
    return true
  }
}
