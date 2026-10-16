import { omit, type Element as SolidElement } from "solid-js"
import type {
  AnchoredProps,
  CodeProps,
  DiffProps,
  HostProps,
  ImgProps,
  InputProps,
  MarkdownProps,
  MotionProps,
  SvgProps,
  TextareaProps,
  VirtualListProps,
} from "@gpuix/native/host"
import { createComponent, createElement, spread } from "./universal.js"
import type { HostElement } from "./host.js"

export type SolidHostProps<Props = HostProps> = Props & {
  children?: SolidElement
  ref?: ((element: HostElement) => void) | HostElement
}

type Component = (props: Record<string, unknown>) => unknown

function normalizeProps(props: Record<string, unknown> | null | undefined) {
  if (!props || !("key" in props)) return props ?? {}
  // omit, not a rest spread, so getters stay reactive.
  return omit(props, "key")
}

export function jsx(
  type: string | Component,
  props?: Record<string, unknown> | null
): SolidElement {
  const normalized = normalizeProps(props)
  if (typeof type === "function") {
    return createComponent(type as never, normalized) as never
  }
  const element = createElement(type)
  spread(element, normalized)
  return element as never
}

export const jsxs = jsx
export const jsxDEV = jsx
export function Fragment(props: { children?: unknown }): SolidElement {
  return (props.children ?? null) as SolidElement
}

/** Solid 2.0 leaves JSX types to the renderer, so GPUIX defines its own. */
export namespace JSX {
  export type Element = SolidElement
  export interface ElementChildrenAttribute { children: {} }
  export interface IntrinsicElements {
    div: SolidHostProps
    text: SolidHostProps
    img: SolidHostProps<ImgProps>
    svg: SolidHostProps<SvgProps>
    canvas: SolidHostProps
    input: SolidHostProps<InputProps>
    textarea: SolidHostProps<TextareaProps>
    anchored: SolidHostProps<AnchoredProps>
    code: SolidHostProps<CodeProps>
    diff: SolidHostProps<DiffProps>
    markdown: SolidHostProps<MarkdownProps>
    "virtual-list": SolidHostProps<VirtualListProps>
  }
}

export type { MotionProps }
