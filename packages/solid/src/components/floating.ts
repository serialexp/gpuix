import {
  createComponent,
  createContext,
  merge,
  omit,
  onSettled,
  Show,
  untrack,
  useContext,
  type Component,
} from "solid-js"
import type { JSX } from "../jsx-runtime.js"
import type { EventPayload } from "@gpuix/native"
import { pushDismissLayer } from "@gpuix/native/host"
import type { DismissLayer, HostProps, KeyEvent, StyleDesc } from "@gpuix/native/host"
import { useGpuixRequired } from "../root.js"
import { isHostElement } from "../host.js"
import { jsx } from "../jsx-runtime.js"
import { spread } from "../universal.js"

export type FloatingSide = "top" | "right" | "bottom" | "left"
export type FloatingAlign = "start" | "center" | "end"
export type StateStyle<State> = StyleDesc | ((state: State) => StyleDesc)

export interface FloatingContentProps extends HostProps {
  children?: JSX.Element
  side?: FloatingSide
  sideOffset?: number
  align?: FloatingAlign
  alignOffset?: number
  collisionPadding?: number
}

export function resolveStyle<State>(
  style: StateStyle<State> | undefined,
  state: State
): StyleDesc | undefined {
  return typeof style === "function" ? style(state) : style
}

export function mergeStyles(
  base: StyleDesc | undefined,
  override: StyleDesc | undefined
): StyleDesc | undefined {
  if (!base) return override
  if (!override) return base
  return { ...base, ...override }
}

export function floatingRootStyle(style?: StyleDesc): StyleDesc {
  return { display: "flex", position: "relative", alignItems: "start", ...style }
}

/** `Show` with element children, which createComponent can't pick out of
 *  the overloads. */
export const ShowWhen = Show as Component<{ when: boolean; children: JSX.Element }>

// Null outside any layer: Solid 2.0 throws for a context with no default.
const DismissLayerContext = createContext<DismissLayer | null>(null)

export interface DismissableLayerProps {
  children?: JSX.Element
  /** Escape reached the window, this layer is on top, and nothing prevented it. */
  onEscapeKeyDown: (event: KeyEvent) => void
  /** Element to focus when the layer opens on top. Null leaves focus. */
  initialFocus?: () => number | null
  /** Element to focus on close. `previous` is the focus from before it opened. */
  finalFocus?: (previous: number | null) => number | null
}

/**
 * Put an open overlay on the window's layer stack while mounted. Escape closes
 * only the top layer. Layers inside this one, such as a Select in a Dialog,
 * are always above it. The stack also moves focus in and out, in stack order.
 */
export function DismissableLayer(props: DismissableLayerProps): JSX.Element {
  const renderer = useGpuixRequired()
  // Read now: Solid has not created this layer's elements yet.
  const previousFocus = renderer.getFocusedElementId?.() ?? null
  const layer: DismissLayer = {
    parent: useContext(DismissLayerContext) ?? undefined,
    onEscapeKeyDown: (event) => props.onEscapeKeyDown(event),
    initialFocus: () => props.initialFocus?.() ?? null,
    finalFocus: (previous) => props.finalFocus?.(previous) ?? null,
  }
  // After the children exist, so initialFocus can name one of them. The
  // returned pop runs when the layer unmounts.
  onSettled(() => pushDismissLayer(renderer, layer, { previousFocus }))
  return createComponent(DismissLayerContext, {
    value: layer,
    get children() { return props.children },
  })
}

export function composeHandlers<Event extends EventPayload>(
  first?: (event: Event) => void,
  second?: (event: Event) => void
) {
  if (!first) return second
  if (!second) return first
  return (event: Event) => {
    first(event)
    second(event)
  }
}

/**
 * Render `props` on a `div`, or spread them onto the single `asChild` element.
 *
 * `defaultTabIndex` makes the part a tab stop, like the `<button>` Base UI
 * renders. An explicit `tabIndex` on the part, then on the child, wins. The
 * child's own handlers run before the part's, like React `asChild`.
 */
export function renderSlot(args: {
  asChild?: boolean
  /** Read lazily, so pass a getter: children that change stay live. */
  readonly children?: JSX.Element
  props: Record<string, unknown>
  defaultTabIndex?: number
}): JSX.Element {
  // Descriptors, not a spread, so reactive getters such as `style` stay live.
  const descriptors = Object.getOwnPropertyDescriptors(args.props)
  const tabIndex = descriptors.tabIndex
  const read = (descriptor: PropertyDescriptor | undefined) =>
    descriptor?.get ? descriptor.get() : descriptor?.value
  if (!args.asChild) {
    if (args.defaultTabIndex !== undefined) {
      descriptors.tabIndex = {
        enumerable: true,
        configurable: true,
        get: () => read(tabIndex) ?? args.defaultTabIndex,
      }
    }
    descriptors.children = {
      enumerable: true,
      configurable: true,
      get: () => args.children,
    }
    return jsx("div", Object.defineProperties({}, descriptors))
  }
  // The child is the element itself, made once.
  const child = untrack(() => args.children)
  if (!isHostElement(child)) {
    throw new Error("asChild requires one GPUIX intrinsic element")
  }
  const childTabIndex = child.props.get("tabIndex")
  if (args.defaultTabIndex !== undefined || childTabIndex !== undefined) {
    descriptors.tabIndex = {
      enumerable: true,
      configurable: true,
      get: () => read(tabIndex) ?? childTabIndex ?? args.defaultTabIndex,
    }
  }
  for (const [key, descriptor] of Object.entries(descriptors)) {
    const own = child.props.get(key)
    if (!key.startsWith("on") || typeof own !== "function") continue
    // A getter too: compiled JSX passes `onX={props.onX}` as one.
    if (!descriptor.get && typeof descriptor.value !== "function") continue
    descriptors[key] = {
      enumerable: true,
      configurable: true,
      value: (event: EventPayload) => {
        own(event)
        const slotHandler = read(descriptor)
        if (typeof slotHandler === "function") slotHandler(event)
      },
    }
  }
  // The child keeps its own children. Without skipChildren, spread would
  // insert `props.children`, which is the child itself.
  delete descriptors.children
  spread(child, Object.defineProperties({}, descriptors), true)
  return child as never
}

export function FloatingLayer(props: FloatingContentProps): JSX.Element {
  const side = () => props.side ?? "bottom"
  return jsx("anchored", {
    get side() { return side() },
    get align() { return props.align ?? "start" },
    get gap() { return props.sideOffset ?? 0 },
    get offset() {
      const alignOffset = props.alignOffset ?? 0
      return side() === "top" || side() === "bottom"
        ? { x: alignOffset, y: 0 }
        : { x: 0, y: alignOffset }
    },
    fit: "snap",
    get snapMargin() { return props.collisionPadding ?? 8 },
    deferred: true,
    priority: 1,
    get occlude() { return props.style?.pointerEvents !== "none" },
    get children() {
      return jsx("div", merge(
        omit(props, "side", "sideOffset", "align", "alignOffset", "collisionPadding", "children"),
        {
          get style() { return mergeStyles({ backgroundColor: "#1A1A1A" }, props.style) },
          get children() { return props.children },
        }
      ))
    },
  })
}
