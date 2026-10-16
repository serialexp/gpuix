import { createSignal, onCleanup, type Accessor } from "solid-js"
import {
  createTextSearchController,
  observeWindowInsets,
  observeWindowSize,
  readWindowInsets,
  readWindowSize,
} from "@gpuix/native/host"
import type {
  ObserverOptions,
  TextSearchOptions,
  WindowInsets,
  WindowSize,
} from "@gpuix/native/host"
import { useGpuix, useGpuixRequired } from "./root.js"

// The observers report the current value as they subscribe, inside the
// component, which Solid 2.0 allows only for signals that opt in.
const OBSERVED = { ownedWrite: true } as const

export function createWindowSize(options: ObserverOptions = {}): Accessor<WindowSize> {
  const renderer = useGpuixRequired()
  const [value, setValue] = createSignal(readWindowSize(renderer), OBSERVED)
  onCleanup(observeWindowSize(renderer, setValue, options))
  return value
}

export function createWindowInsets(
  options: ObserverOptions = {}
): Accessor<WindowInsets> {
  const renderer = useGpuixRequired()
  const [value, setValue] = createSignal(readWindowInsets(renderer), OBSERVED)
  onCleanup(observeWindowInsets(renderer, setValue, options))
  return value
}

export function createSelectedText(): Accessor<string | null> {
  const context = useGpuix()
  if (!context) throw new Error("createSelectedText must be used inside a GPUIX root")
  const [value, setValue] = createSignal(context.renderer.getSelectedText?.() ?? null, OBSERVED)
  onCleanup(context.subscribeSelection(setValue))
  return value
}

export function createTextSearch(options: Accessor<TextSearchOptions>) {
  const controller = createTextSearchController()
  const [revision, setRevision] = createSignal(0)
  onCleanup(controller.subscribe(() => setRevision((value) => value + 1)))
  return {
    get props() {
      revision()
      return controller.getSnapshot(options()).props
    },
    get total() {
      revision()
      return controller.getSnapshot(options()).total
    },
    get active() {
      revision()
      return controller.getSnapshot(options()).active
    },
    next: () => controller.next(),
    previous: () => controller.previous(),
    goTo: (index: number) => controller.goTo(index),
  }
}
