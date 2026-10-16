import type { JSX } from "./jsx-runtime.js"
import {
  TestRenderer,
  hasNativeTestRenderer,
  type TestRendererOptions,
} from "@gpuix/native/testing"
import { createRoot, type Root } from "./root.js"

export * from "@gpuix/native/testing"

export interface TestRoot {
  root: Root
  renderer: TestRenderer
  render(code: () => JSX.Element): void
  flushSync<Value>(fn: () => Value): Value
  unmount(): void
}

export function createTestRoot(options: TestRendererOptions = {}): TestRoot {
  let root!: Root
  const renderer = new TestRenderer({
    ...options,
    dispatchEvent: (event) => root.dispatch(event),
  })
  root = createRoot(renderer)
  return {
    root,
    renderer,
    render(code) {
      root.render(code)
      renderer.flush()
    },
    flushSync(fn) {
      const value = root.flushSync(fn)
      renderer.flush()
      return value
    },
    unmount() {
      root.unmount()
      renderer.flush()
    },
  }
}

export { TestRenderer, hasNativeTestRenderer }
