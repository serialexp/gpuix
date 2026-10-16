// `@gpuix/native` in the embedded QuickJS runtime. There is no napi addon:
// the Rust host owns the window and drives the run loop, and installs
// `__gpuix` before the app runs. This renderer forwards batches to the
// window's tree, its window methods to the host (`src/quickjs/window.rs`),
// and the host's events to the callback, as the addon does.
import type {
  DebugFrameOverlayStats,
  ElementBounds,
  EventPayload,
  GpuixRenderer as NapiRenderer,
  HighlightMatch,
  PathPromptOptions,
  WindowInsets,
  WindowOptions,
  WindowSize,
} from "../index.js"

interface EmbeddedHost {
  init(json: string): void
  renderer: {
    applyBatch(json: string): number[]
    window(method: string, ...args: unknown[]): unknown
    promptForPaths(options?: PathPromptOptions | null): Promise<string[] | null>
  }
  setEventHandler(handler: ((event: EventPayload) => unknown) | undefined): void
}

declare const __gpuix: EmbeddedHost

/** The napi methods this renderer has too, so their signatures can't drift. */
type WindowMethods = Pick<
  NapiRenderer,
  | "getWindowSize"
  | "getWindowInsets"
  | "setDebugFrameOverlay"
  | "cycleDebugFrameOverlay"
  | "getDebugFrameOverlay"
  | "resetDebugFrameOverlayStats"
  | "getDebugFrameOverlayStats"
  | "activateWindow"
  | "minimizeWindow"
  | "zoomWindow"
  | "toggleFullscreen"
  | "setWindowTitle"
  | "promptForPaths"
  | "focusElement"
  | "focusNext"
  | "focusPrevious"
  | "getFocusedElementId"
  | "focusNextWithin"
  | "focusPreviousWithin"
  | "setWindowKeyEvents"
  | "blur"
  | "setWindowSelectionChange"
  | "getSelectedText"
  | "clearSelection"
  | "scrollTo"
  | "scrollToItem"
  | "getListScrollTop"
  | "getScrollOffset"
  | "scrollIntoView"
  | "setImagePixels"
  | "setImage"
  | "registerFont"
  | "getElementBounds"
  | "getElementsAt"
  | "getPaintedHighlights"
>

function call<T = void>(method: string, ...args: unknown[]): T {
  return __gpuix.renderer.window(method, ...args) as T
}

export class GpuixRenderer implements WindowMethods {
  constructor(eventCallback?: ((err: Error | null, event: EventPayload) => unknown) | null) {
    __gpuix.setEventHandler(eventCallback ? (event) => eventCallback(null, event) : undefined)
  }

  /** The host opens the window with these options once the app has loaded. */
  init(options?: WindowOptions | null): void {
    __gpuix.init(JSON.stringify(options ?? {}))
  }

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

  getWindowSize(): WindowSize {
    return call("getWindowSize")
  }

  getWindowInsets(): WindowInsets {
    return call("getWindowInsets")
  }

  setDebugFrameOverlay(mode: string): string {
    return call("setDebugFrameOverlay", mode)
  }

  cycleDebugFrameOverlay(): string {
    return call("cycleDebugFrameOverlay")
  }

  getDebugFrameOverlay(): string {
    return call("getDebugFrameOverlay")
  }

  resetDebugFrameOverlayStats(): void {
    call("resetDebugFrameOverlayStats")
  }

  getDebugFrameOverlayStats(): DebugFrameOverlayStats {
    return call("getDebugFrameOverlayStats")
  }

  activateWindow(): void {
    call("activateWindow")
  }

  minimizeWindow(): void {
    call("minimizeWindow")
  }

  zoomWindow(): void {
    call("zoomWindow")
  }

  toggleFullscreen(): void {
    call("toggleFullscreen")
  }

  setWindowTitle(title: string): void {
    call("setWindowTitle", title)
  }

  promptForPaths(options?: PathPromptOptions | null): Promise<string[] | null> {
    return __gpuix.renderer.promptForPaths(options)
  }

  focusElement(elementId: number): void {
    call("focusElement", elementId)
  }

  focusNext(): void {
    call("focusNext")
  }

  focusPrevious(): void {
    call("focusPrevious")
  }

  getFocusedElementId(): number | null {
    return call("getFocusedElementId")
  }

  focusNextWithin(elementId: number): void {
    call("focusNextWithin", elementId)
  }

  focusPreviousWithin(elementId: number): void {
    call("focusPreviousWithin", elementId)
  }

  setWindowKeyEvents(keyDown: boolean, keyUp: boolean, eventId: number): void {
    call("setWindowKeyEvents", keyDown, keyUp, eventId)
  }

  blur(): void {
    call("blur")
  }

  setWindowSelectionChange(enabled: boolean, eventId: number): void {
    call("setWindowSelectionChange", enabled, eventId)
  }

  getSelectedText(): string | null {
    return call("getSelectedText")
  }

  clearSelection(): void {
    call("clearSelection")
  }

  scrollTo(elementId: number, x: number, y: number): void {
    call("scrollTo", elementId, x, y)
  }

  scrollToItem(elementId: number, index: number, offsetInItem?: number | null): void {
    call("scrollToItem", elementId, index, offsetInItem)
  }

  getListScrollTop(elementId: number): number[] | null {
    return call("getListScrollTop", elementId)
  }

  getScrollOffset(elementId: number): number[] | null {
    return call("getScrollOffset", elementId)
  }

  scrollIntoView(elementId: number): void {
    call("scrollIntoView", elementId)
  }

  setImagePixels(
    elementId: number,
    width: number,
    height: number,
    pixels: Uint8Array,
    format?: string | null,
  ): void {
    call("setImagePixels", elementId, width, height, pixels, format)
  }

  setImage(elementId: number, bytes: Uint8Array): void {
    call("setImage", elementId, bytes)
  }

  registerFont(data: Uint8Array): void {
    call("registerFont", data)
  }

  getElementBounds(id: number): ElementBounds | null {
    return call("getElementBounds", id)
  }

  getElementsAt(x: number, y: number): number[] {
    return call("getElementsAt", x, y)
  }

  getPaintedHighlights(): HighlightMatch[] {
    return call("getPaintedHighlights")
  }
}
