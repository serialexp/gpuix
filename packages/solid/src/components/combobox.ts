import {
  createComponent,
  createContext,
  createSignal,
  For,
  merge,
  omit,
  createRenderEffect,
  untrack,
  useContext,
  type Accessor,
  type Component,
} from "solid-js"
import type { JSX } from "../jsx-runtime.js"
import type { EventPayload } from "@gpuix/native"
import type { HostProps, InputProps, KeyEvent } from "@gpuix/native/host"
import type { HostElement } from "../host.js"
import { jsx } from "../jsx-runtime.js"
import { useGpuixRequired } from "../root.js"
import {
  FloatingLayer,
  floatingRootStyle,
  renderSlot,
  resolveStyle,
  type FloatingContentProps,
  type StateStyle,
  DismissableLayer,
  ShowWhen,
} from "./floating.js"

export type ComboboxValue = string | string[] | null
interface ComboContext {
  open(): boolean
  value(): ComboboxValue
  input(): string
  active(): number | null
  filtered(): string[]
  disabled(): boolean
  multiple(): boolean
  inputRef: { current: HostElement | null }
  setOpen(value: boolean): void
  setInput(value: string): void
  setActive(value: number | null): void
  move(delta: number): void
  select(value: string): void
  registerDisabled(value: string, disabled: boolean): () => void
}
/** `For` with its keyed child shape, which createComponent can't pick out of
 *  the overloads. */
const KeyedFor = For as Component<{
  each: readonly string[]
  children: (item: string, index: Accessor<number>) => JSX.Element
}>

// Null outside a Combobox, so the parts can say what they need.
const ComboboxContext = createContext<ComboContext | null>(null)
function context(name: string): ComboContext {
  const value = useContext(ComboboxContext)
  if (!value) throw new Error(`${name} must be used inside Combobox`)
  return value
}

export interface ComboboxProps extends HostProps {
  children?: JSX.Element
  items?: readonly string[]
  value?: ComboboxValue
  defaultValue?: ComboboxValue
  onValueChange?: (value: ComboboxValue) => void
  inputValue?: string
  defaultInputValue?: string
  onInputValueChange?: (value: string) => void
  open?: boolean
  defaultOpen?: boolean
  onOpenChange?: (open: boolean) => void
  multiple?: boolean
  disabled?: boolean
  autoHighlight?: boolean | "always"
  filter?: null | ((item: string, query: string, itemToString: (item: string) => string) => boolean)
  itemToStringValue?: (item: string) => string
}

export function Combobox(props: ComboboxProps): JSX.Element {
  const renderer = useGpuixRequired()
  const [internalValue, setInternalValue] = createSignal<ComboboxValue>(
    untrack(() => props.defaultValue ?? null)
  )
  const [internalInput, setInternalInput] = createSignal(untrack(() => props.defaultInputValue ?? ""))
  const [internalOpen, setInternalOpen] = createSignal(untrack(() => props.defaultOpen ?? false))
  const [active, setActive] = createSignal<number | null>(null)
  const inputRef: { current: HostElement | null } = { current: null }
  const disabledItems = new Set<string>()
  const value = () => props.value === undefined ? internalValue() : props.value
  const input = () => props.inputValue === undefined ? internalInput() : props.inputValue
  const open = () => props.open === undefined ? internalOpen() : props.open
  const itemToString = (item: string) => props.itemToStringValue?.(item) ?? item
  const filter = (text: string) => {
    const query = text.trim().toLowerCase()
    if (props.filter === null) return [...(props.items ?? [])]
    if (props.filter) return [...(props.items ?? [])].filter((item) => props.filter!(item, query, itemToString))
    return [...(props.items ?? [])].filter((item) => itemToString(item).toLowerCase().includes(query))
  }
  const filtered = () => filter(input())
  const setOpen = (next: boolean) => {
    const previous = open()
    if (props.open === undefined) setInternalOpen(next)
    if (previous !== next) props.onOpenChange?.(next)
    if (next) {
      queueMicrotask(() => {
        if (inputRef.current) renderer.focusElement?.(inputRef.current.id)
      })
    }
  }
  const setInput = (next: string) => {
    const previous = input()
    if (props.inputValue === undefined) setInternalInput(next)
    if (previous !== next) props.onInputValueChange?.(next)
    // From `next`: Solid 2.0 shows the write to `input()` only after a flush.
    const nextItems = filter(props.inputValue === undefined ? next : props.inputValue)
    const firstEnabled = nextItems.findIndex((item) => !disabledItems.has(item))
    setActive(props.autoHighlight && firstEnabled >= 0 ? firstEnabled : null)
  }
  const setValue = (next: ComboboxValue) => {
    const previous = value()
    if (props.value === undefined) setInternalValue(next)
    if (!Object.is(previous, next)) props.onValueChange?.(next)
  }
  const move = (delta: number) => {
    const items = filtered()
    if (items.length === 0) return
    let next = active() === null ? (delta > 0 ? -1 : 0) : active()!
    for (let checked = 0; checked < items.length; checked += 1) {
      next = (next + delta + items.length) % items.length
      if (!disabledItems.has(items[next]!)) {
        setActive(next)
        return
      }
    }
  }
  const select = (item: string) => {
    if (props.disabled || disabledItems.has(item)) return
    if (props.multiple) {
      const current = value()
      const selected = Array.isArray(current) ? current : []
      setValue(selected.includes(item) ? selected.filter((value) => value !== item) : [...selected, item])
      setInput("")
      return
    }
    setValue(item)
    setInput(itemToString(item))
    setOpen(false)
  }
  const state: ComboContext = {
    open,
    value,
    input,
    active,
    filtered,
    disabled: () => props.disabled ?? false,
    multiple: () => props.multiple ?? false,
    inputRef,
    setOpen,
    setInput,
    setActive,
    move,
    select,
    registerDisabled(item, disabled) {
      if (disabled) disabledItems.add(item)
      return () => { disabledItems.delete(item) }
    },
  }
  return createComponent(ComboboxContext, {
    value: state,
    get children() {
      return jsx("div", merge(
        omit(
          props,
          "items",
          "value",
          "defaultValue",
          "onValueChange",
          "inputValue",
          "defaultInputValue",
          "onInputValueChange",
          "open",
          "defaultOpen",
          "onOpenChange",
          "multiple",
          "autoHighlight",
          "filter",
          "itemToStringValue",
          "children"
        ),
        {
          get style() { return floatingRootStyle(props.style) },
          get children() { return props.children },
        }
      ))
    },
  })
}

export interface ComboboxInputProps extends InputProps {
  disabled?: boolean
  ref?: (element: HostElement) => void
}
export function ComboboxInput(props: ComboboxInputProps): JSX.Element {
  const state = context("ComboboxInput")
  const disabled = () => props.disabled ?? state.disabled()
  return jsx("input", merge(props, {
    ref(element: HostElement) {
      state.inputRef.current = element
      props.ref?.(element)
    },
    get value() { return state.input() },
    get readOnly() { return disabled() || props.readOnly },
    get autoFocus() { return state.open() },
    onClick(event: EventPayload) {
      props.onClick?.(event)
      if (!disabled()) state.setOpen(true)
    },
    onFocus(event: EventPayload) {
      props.onFocus?.(event)
      if (!disabled()) state.setOpen(true)
    },
    // Focus leaving the input closes the popup, like Base UI. Tab moves focus
    // only when no handler prevented it, so a kept Tab keeps it open.
    onBlur(event: EventPayload) {
      props.onBlur?.(event)
      state.setOpen(false)
    },
    onChange(event: EventPayload) {
      props.onChange?.(event)
      state.setInput(event.value ?? "")
      if (!disabled()) state.setOpen(true)
    },
    onKeyDown(event: KeyEvent) {
      props.onKeyDown?.(event)
      if (event.key === "down") state.move(1)
      else if (event.key === "up") state.move(-1)
    },
    onSubmit(event: EventPayload) {
      props.onSubmit?.(event)
      const index = state.active()
      if (index !== null) {
        const item = state.filtered().at(index)
        if (item !== undefined) state.select(item)
      }
    },
  }))
}

export interface ComboboxTriggerProps extends HostProps { children?: JSX.Element; asChild?: boolean; disabled?: boolean }
export function ComboboxTrigger(props: ComboboxTriggerProps): JSX.Element {
  const state = context("ComboboxTrigger")
  const disabled = () => props.disabled ?? state.disabled()
  return renderSlot({
    asChild: props.asChild,
    get children() { return props.children },
    props: merge(omit(props, "asChild", "children"), {
      get tabIndex() { return disabled() ? -1 : props.tabIndex },
      onClick(event: EventPayload) {
        props.onClick?.(event)
        if (!disabled()) state.setOpen(!state.open())
      },
    }),
    defaultTabIndex: 0,
  })
}

export interface ComboboxValueProps extends HostProps {
  children?: JSX.Element | ((value: ComboboxValue) => JSX.Element)
  placeholder?: JSX.Element
}
export function ComboboxValue(props: ComboboxValueProps): JSX.Element {
  const state = context("ComboboxValue")
  return jsx("div", merge(omit(props, "placeholder", "children"), {
    get children() {
      if (typeof props.children === "function") return props.children(state.value())
      if (props.children !== undefined) return props.children
      const value = state.value()
      return Array.isArray(value) ? value.join(", ") : (value ?? props.placeholder)
    },
  }))
}

export function ComboboxContent(props: FloatingContentProps): JSX.Element {
  const state = context("ComboboxContent")
  return createComponent(ShowWhen, {
    get when() { return state.open() },
    get children() {
      return createComponent(DismissableLayer, {
        onEscapeKeyDown: () => state.setOpen(false),
        get children() {
          return FloatingLayer(merge(props, {
            onMouseDownOutside(event: EventPayload) {
              props.onMouseDownOutside?.(event)
              state.setOpen(false)
            },
          }))
        },
      })
    },
  })
}

export interface ComboboxListProps extends HostProps { children?: JSX.Element | ((item: string) => JSX.Element) }
export function ComboboxList(props: ComboboxListProps): JSX.Element {
  const state = context("ComboboxList")
  return jsx("div", merge(omit(props, "children"), {
    get children() {
      const render = props.children
      if (typeof render !== "function") return render
      return createComponent(KeyedFor, { get each() { return state.filtered() }, children: render })
    },
  }))
}

export interface ComboboxItemState { selected: boolean; highlighted: boolean; disabled: boolean }
export interface ComboboxItemProps extends Omit<HostProps, "style"> {
  value: string
  disabled?: boolean
  asChild?: boolean
  children?: JSX.Element | ((state: ComboboxItemState) => JSX.Element)
  style?: StateStyle<ComboboxItemState>
}
export function ComboboxItem(props: ComboboxItemProps): JSX.Element {
  const state = context("ComboboxItem")
  createRenderEffect(
    () => [props.value, props.disabled ?? false] as const,
    ([value, disabled]) => state.registerDisabled(value, disabled)
  )
  const itemState = () => ({
    selected: (() => {
      const value = state.value()
      return Array.isArray(value) ? value.includes(props.value) : value === props.value
    })(),
    highlighted: state.filtered().indexOf(props.value) === state.active(),
    disabled: props.disabled ?? false,
  })
  return renderSlot({
    asChild: props.asChild,
    get children() { return typeof props.children === "function" ? props.children(itemState()) : props.children },
    props: merge(omit(props, "asChild", "children"), {
      get style() { return resolveStyle(props.style, itemState()) },
      onMouseEnter(event: EventPayload) {
        props.onMouseEnter?.(event)
        if (!itemState().disabled) state.setActive(state.filtered().indexOf(props.value))
      },
      onClick(event: EventPayload) {
        props.onClick?.(event)
        if (!itemState().disabled) state.select(props.value)
      },
    }),
  })
}

export function ComboboxEmpty(props: HostProps & { children?: JSX.Element }): JSX.Element {
  const state = context("ComboboxEmpty")
  return createComponent(ShowWhen, {
    get when() { return state.filtered().length === 0 },
    get children() { return jsx("div", props as Record<string, unknown>) },
  })
}
const div = (props: HostProps & { children?: JSX.Element }) => jsx("div", props as Record<string, unknown>)
export const ComboboxGroup = div
export const ComboboxLabel = div
export const ComboboxSeparator = div

export {
  Combobox as Root,
  ComboboxContent as Content,
  ComboboxEmpty as Empty,
  ComboboxGroup as Group,
  ComboboxInput as Input,
  ComboboxItem as Item,
  ComboboxLabel as Label,
  ComboboxList as List,
  ComboboxSeparator as Separator,
  ComboboxTrigger as Trigger,
  ComboboxValue as Value,
}
