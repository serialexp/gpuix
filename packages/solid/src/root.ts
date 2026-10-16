import { createComponent, createContext, flush as flushSolid, useContext } from "solid-js"
import type { JSX } from "./jsx-runtime.js"
import type { EventPayload } from "@gpuix/native"
import {
  createMutationQueue,
  createRendererState,
  unregisterEventHandlers,
} from "@gpuix/native/host"
import type {
  NativeRenderer,
  RendererRootBinding,
  RootEventHandlers,
} from "@gpuix/native/host"
import { HostRoot } from "./host.js"
import { universalRender } from "./universal.js"

export interface GpuixContextValue {
  renderer: NativeRenderer
  subscribeSelection(callback: (text: string | null) => void): () => void
}

// Null outside a root: Solid 2.0 throws for a context with no default.
export const GpuixContext = createContext<GpuixContextValue | null>(null)

export function useGpuix(): GpuixContextValue | undefined {
  return useContext(GpuixContext) ?? undefined
}

export function useGpuixRequired(): NativeRenderer {
  const value = useGpuix()
  if (!value) throw new Error("useGpuixRequired must be used inside a GPUIX root")
  return value.renderer
}

/** Where an error nothing caught goes when the app gave no
 *  `onUncaughtError`: the console, as `@gpuix/react`'s default does. Not
 *  `reportError`, which ends a Bun process. */
export function reportUncaughtError(error: unknown): void {
  console.error("[gpuix-solid] uncaught error", error)
}

export interface Root {
  render(code: () => JSX.Element): void
  flush(): void
  flushSync<Value>(fn: () => Value): Value
  dispatch(event: EventPayload): boolean
  unmount(): void
}

export function createRoot(
  renderer: NativeRenderer,
  rootHandlers: RootEventHandlers = {}
): Root {
  const state = createRendererState(renderer)
  const eventHandlers = new Map()
  const mutations = createMutationQueue(renderer, (ids) => {
    for (const id of ids) unregisterEventHandlers(eventHandlers, id)
  }, (flush) => {
    queueMicrotask(() => {
      try {
        flush()
      } catch (error) {
        if (!rootHandlers.onUncaughtError) reportUncaughtError(error)
        else rootHandlers.onUncaughtError(error instanceof Error ? error : new Error(String(error)), {})
      }
    })
  })
  const container = new HostRoot({
    mutations,
    nativeRenderer: renderer,
    eventHandlers,
    allocateId: () => ++state.ids.nextElementId,
  })
  let binding: RendererRootBinding | undefined
  let dispose: (() => void) | undefined
  const selectionListeners = new Set<(text: string | null) => void>()
  const onSelectionChange = (event: EventPayload, currentRenderer: NativeRenderer) => {
    rootHandlers.onSelectionChange?.(event, currentRenderer)
    for (const listener of selectionListeners) listener(event.value ?? null)
  }
  const syncSelection = () => {
    if (!binding) return
    renderer.setWindowSelectionChange?.(
      Boolean(rootHandlers.onSelectionChange) || selectionListeners.size > 0,
      binding.windowSelectionEventId
    )
  }

  const attach = () => {
    binding = state.attach({
      eventHandlers,
      onWindowKeyDown: rootHandlers.onKeyDown,
      onWindowKeyUp: rootHandlers.onKeyUp,
      onSelectionChange,
      tabNavigation: rootHandlers.tabNavigation,
      onEvent: rootHandlers.onEvent,
      // A handler's writes reach the native tree before the event returns, as
      // a controlled input needs. Native delivers events from its own loop,
      // never from inside a Solid flush, so this flush is never nested.
      schedule(dispatch) {
        try {
          dispatch()
        } finally {
          flushAll()
        }
      },
    })
    renderer.setWindowKeyEvents?.(
      // Always on: the Tab and Escape defaults run on the window event.
      true,
      Boolean(rootHandlers.onKeyUp),
      binding.windowKeyEventId
    )
    // Sent on every attach, so a remount without the option restores the default.
    mutations.setKeyboardFocusDim(rootHandlers.keyboardFocusDim !== false)
    syncSelection()
  }

  const detach = () => {
    if (!binding?.detach()) return
    renderer.setWindowKeyEvents?.(false, false, binding.windowKeyEventId)
    renderer.setWindowSelectionChange?.(false, binding.windowSelectionEventId)
    binding = undefined
  }

  const flush = () => mutations.flushMutations()
  // Solid 2.0 applies writes on the next microtask; this applies them now.
  const flushAll = () => {
    flushSolid()
    flush()
  }
  attach()
  const clear = () => {
    // Disposing also removes what the render mounted.
    dispose?.()
    dispose = undefined
    flush()
    eventHandlers.clear()
  }

  return {
    render(code) {
      if (dispose) {
        clear()
        detach()
        attach()
      }
      dispose = universalRender(
        () => createComponent(GpuixContext, {
          value: {
            renderer,
            subscribeSelection(callback) {
              selectionListeners.add(callback)
              callback(renderer.getSelectedText?.() ?? null)
              syncSelection()
              return () => {
                selectionListeners.delete(callback)
                syncSelection()
              }
            },
          },
          get children() {
            return code()
          },
        }),
        container
      )
      flush()
    },
    flush: flushAll,
    flushSync(fn) {
      try {
        return flushSolid(fn)
      } finally {
        flush()
      }
    },
    dispatch: (event) => state.dispatch(event),
    unmount() {
      clear()
      detach()
    },
  }
}
