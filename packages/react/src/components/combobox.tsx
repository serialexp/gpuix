/** Headless shadcn-shaped Combobox components with native GPUI text input. */

import React, {
  createContext,
  forwardRef,
  useContext,
  useRef,
  useState,
} from "react"
import type { ReactElement, ReactNode } from "react"
import type { EventPayload } from "@gpuix/native"
import type { InputProps, Props, PublicInstance } from "../types/host.js"
import { useGpuix } from "../hooks/use-gpuix.js"
import {
  FloatingLayer,
  floatingRootStyle,
  renderSlot,
  resolveStyle,
  setRefs,
  useControllableState,
} from "./floating.js"
import type { FloatingContentProps, StateStyle } from "./floating.js"

export type ComboboxValue = string | string[] | null

interface ComboboxContextValue {
  open: boolean
  disabled: boolean
  multiple: boolean
  value: ComboboxValue
  inputValue: string
  filteredItems: readonly string[]
  activeIndex: number | null
  inputRef: React.MutableRefObject<PublicInstance | null>
  itemToString: (item: string) => string
  setOpen: (open: boolean) => void
  setInputValue: (value: string) => void
  setActiveIndex: (index: number | null) => void
  moveActive: (delta: number) => void
  selectItem: (item: string) => void
  registerItem: (item: { value: string; disabled: boolean; mounted: boolean }) => void
}

const ComboboxContext = createContext<ComboboxContextValue | null>(null)

function useComboboxContext(name: string): ComboboxContextValue {
  const context = useContext(ComboboxContext)
  if (!context) throw new Error(`${name} must be used inside Combobox`)
  return context
}

function defaultFilter({
  items,
  query,
  itemToString,
}: {
  items: readonly string[]
  query: string
  itemToString: (item: string) => string
}): string[] {
  const normalized = query.trim().toLowerCase()
  if (!normalized) return [...items]
  const matches: Array<{ item: string; rank: number; index: number }> = []
  items.forEach((item, index) => {
    const label = itemToString(item).toLowerCase()
    const rank = label.startsWith(normalized) ? 0 : label.includes(normalized) ? 1 : null
    if (rank !== null) matches.push({ item, rank, index })
  })
  return matches
    .sort((left, right) => left.rank - right.rank || left.index - right.index)
    .map((match) => match.item)
}

export interface ComboboxProps extends Omit<Props, "children" | "onChange"> {
  children?: ReactNode
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

export function Combobox({
  children,
  items = [],
  value: valueProp,
  defaultValue = null,
  onValueChange,
  inputValue: inputValueProp,
  defaultInputValue = "",
  onInputValueChange,
  open: openProp,
  defaultOpen = false,
  onOpenChange,
  multiple = false,
  disabled = false,
  autoHighlight = false,
  filter,
  itemToStringValue = (item) => item,
  style,
  ...props
}: ComboboxProps): ReactElement {
  const { renderer } = useGpuix()
  const [value, setValue] = useControllableState<ComboboxValue>({
    value: valueProp,
    defaultValue,
    onChange: onValueChange,
  })
  const [inputValue, setInputValueState] = useControllableState({
    value: inputValueProp,
    defaultValue: defaultInputValue,
    onChange: onInputValueChange,
  })
  const [open, setOpenState] = useControllableState({
    value: openProp,
    defaultValue: defaultOpen,
    onChange: onOpenChange,
  })
  const [activeIndex, setActiveIndex] = useState<number | null>(null)
  const inputRef = useRef<PublicInstance | null>(null)
  const disabledItems = useRef<string[]>([])
  const itemToString = itemToStringValue

  const filterItems = (query: string): string[] => {
    if (filter === null) return [...items]
    if (filter) {
      return items.filter((item) => filter(item, query, itemToStringValue))
    }
    return defaultFilter({ items, query, itemToString })
  }
  const filteredItems = filterItems(inputValue)

  const setOpen = (nextOpen: boolean) => {
    setOpenState(nextOpen)
    if (nextOpen) {
      queueMicrotask(() => {
        if (inputRef.current) renderer?.focusElement?.(inputRef.current.id)
      })
    }
  }

  const registerItem = ({ value: item, disabled: itemDisabled, mounted }: {
    value: string
    disabled: boolean
    mounted: boolean
  }) => {
    disabledItems.current = disabledItems.current.filter((candidate) => candidate !== item)
    if (mounted && itemDisabled) disabledItems.current.push(item)
  }

  const updateInputValue = (nextValue: string) => {
    setInputValueState(nextValue)
    const nextItems = filterItems(nextValue)
    const firstEnabled = nextItems.findIndex((item) => !disabledItems.current.includes(item))
    setActiveIndex(autoHighlight && firstEnabled >= 0 ? firstEnabled : null)
  }

  const moveActive = (delta: number) => {
    if (filteredItems.length === 0) return
    let nextIndex = activeIndex === null ? (delta > 0 ? -1 : 0) : activeIndex
    for (let checked = 0; checked < filteredItems.length; checked++) {
      nextIndex = (nextIndex + delta + filteredItems.length) % filteredItems.length
      if (!disabledItems.current.includes(filteredItems[nextIndex])) {
        setActiveIndex(nextIndex)
        return
      }
    }
  }

  const selectItem = (item: string) => {
    if (disabled || disabledItems.current.includes(item)) return
    if (multiple) {
      const selected = Array.isArray(value) ? value : []
      const exists = selected.includes(item)
      setValue(exists ? selected.filter((candidate) => candidate !== item) : [...selected, item])
      setInputValueState("")
      setActiveIndex(null)
      return
    }
    setValue(item)
    setInputValueState(itemToString(item))
    setOpen(false)
    setActiveIndex(null)
  }

  const context: ComboboxContextValue = {
    open,
    disabled,
    multiple,
    value,
    inputValue,
    filteredItems,
    activeIndex,
    inputRef,
    itemToString,
    setOpen,
    setInputValue: updateInputValue,
    setActiveIndex,
    moveActive,
    selectItem,
    registerItem,
  }

  return (
    <ComboboxContext.Provider value={context}>
      <div {...props} style={floatingRootStyle(style)}>{children}</div>
    </ComboboxContext.Provider>
  )
}

export interface ComboboxInputProps extends InputProps {
  disabled?: boolean
}

export const ComboboxInput = forwardRef<PublicInstance, ComboboxInputProps>(
  function ComboboxInput(
    { onBlur, onChange, onClick, onFocus, onKeyDown, onKeyUp, onSubmit, disabled: disabledProp, ...props },
    forwardedRef
  ) {
    const context = useComboboxContext("ComboboxInput")
    const disabled = disabledProp ?? context.disabled
    const ref = (value: PublicInstance | null) => {
      context.inputRef.current = value
      setRefs(value, forwardedRef)
    }
    return (
      <input
        {...props}
        ref={ref}
        value={context.inputValue}
        readOnly={disabled || props.readOnly}
        autoFocus={context.open}
        onClick={(event: EventPayload) => {
          onClick?.(event)
          if (!disabled) context.setOpen(true)
        }}
        onFocus={(event: EventPayload) => {
          onFocus?.(event)
          if (!disabled) context.setOpen(true)
        }}
        onBlur={(event: EventPayload) => {
          onBlur?.(event)
          context.setOpen(false)
        }}
        onChange={(event: EventPayload) => {
          onChange?.(event)
          context.setInputValue(event.value ?? "")
          if (!disabled) context.setOpen(true)
        }}
        onKeyDown={(event: EventPayload) => {
          onKeyDown?.(event)
          if (disabled) return
          if (event.key === "escape" || event.key === "tab") {
            context.setOpen(false)
          } else if (event.key === "down" || (event.key === "n" && event.modifiers?.ctrl)) {
            context.moveActive(1)
          } else if (event.key === "up" || (event.key === "p" && event.modifiers?.ctrl)) {
            context.moveActive(-1)
          }
        }}
        onKeyUp={(event: EventPayload) => {
          onKeyUp?.(event)
        }}
        onSubmit={(event: EventPayload) => {
          onSubmit?.(event)
          if (disabled) return
          if (context.activeIndex !== null) {
            const item = context.filteredItems[context.activeIndex]
            if (item !== undefined) context.selectItem(item)
          }
        }}
      />
    )
  }
)

export interface ComboboxTriggerProps extends Props {
  asChild?: boolean
  disabled?: boolean
}

export const ComboboxTrigger = forwardRef<PublicInstance, ComboboxTriggerProps>(
  function ComboboxTrigger(
    { asChild, disabled: disabledProp, children, onClick, onKeyDown, ...props },
    ref
  ) {
    const context = useComboboxContext("ComboboxTrigger")
    const disabled = disabledProp ?? context.disabled
    return renderSlot({
      asChild,
      children,
      props: {
        ...props,
        tabIndex: disabled ? -1 : (asChild ? props.tabIndex : (props.tabIndex ?? 0)),
        onClick: (event) => {
          onClick?.(event)
          if (!disabled) context.setOpen(!context.open)
        },
        onKeyDown: (event) => {
          onKeyDown?.(event)
          if (disabled) return
          if (event.key === "down" || event.key === "up") context.setOpen(true)
          if (event.key === "escape") context.setOpen(false)
        },
      },
      ref
    })
  }
)

export interface ComboboxValueProps extends Omit<Props, "children"> {
  placeholder?: ReactNode
  children?: ReactNode | ((value: ComboboxValue) => ReactNode)
}

export const ComboboxValue = forwardRef<PublicInstance, ComboboxValueProps>(
  function ComboboxValue({ placeholder, children, ...props }, ref) {
    const context = useComboboxContext("ComboboxValue")
    const value = Array.isArray(context.value)
      ? context.value.map(context.itemToString).join(", ")
      : context.value === null
        ? ""
        : context.itemToString(context.value)
    const content = typeof children === "function" ? children(context.value) : children
    return <div {...props} ref={ref}>{content ?? (value || placeholder)}</div>
  }
)

export const ComboboxContent = forwardRef<PublicInstance, FloatingContentProps>(
  function ComboboxContent({ children, onMouseDownOutside, ...props }, ref) {
    const context = useComboboxContext("ComboboxContent")
    if (!context.open) return null
    return (
      <FloatingLayer
        {...props}
        ref={ref}
        onMouseDownOutside={(event) => {
          onMouseDownOutside?.(event)
          context.setOpen(false)
        }}
      >
        {children}
      </FloatingLayer>
    )
  }
)

export interface ComboboxListProps extends Omit<Props, "children"> {
  children?: ReactNode | ((item: string) => ReactNode)
}

export const ComboboxList = forwardRef<PublicInstance, ComboboxListProps>(
  function ComboboxList({ children, ...props }, ref) {
    const context = useComboboxContext("ComboboxList")
    return (
      <div {...props} ref={ref}>
        {typeof children === "function"
          ? context.filteredItems.map((item) => children(item))
          : children}
      </div>
    )
  }
)

export interface ComboboxItemState {
  selected: boolean
  highlighted: boolean
  disabled: boolean
}

export interface ComboboxItemProps extends Omit<Props, "children" | "style"> {
  value: string
  disabled?: boolean
  children?: ReactNode | ((state: ComboboxItemState) => ReactNode)
  style?: StateStyle<ComboboxItemState>
}

export const ComboboxItem = forwardRef<PublicInstance, ComboboxItemProps>(
  function ComboboxItem(
    { value, disabled = false, children, style, onClick, onMouseEnter, ...props },
    ref
  ) {
    const context = useComboboxContext("ComboboxItem")
    const index = context.filteredItems.indexOf(value)
    const selected = Array.isArray(context.value)
      ? context.value.includes(value)
      : context.value === value
    const state = { selected, highlighted: context.activeIndex === index, disabled }
    const itemRef = (instance: PublicInstance | null) => {
      context.registerItem({ value, disabled, mounted: instance !== null })
      setRefs(instance, ref)
    }
    return (
      <div
        {...props}
        ref={itemRef}
        style={resolveStyle(style, state)}
        onMouseEnter={(event: EventPayload) => {
          onMouseEnter?.(event)
          if (!disabled && index >= 0) context.setActiveIndex(index)
        }}
        onClick={(event: EventPayload) => {
          onClick?.(event)
          if (!disabled) context.selectItem(value)
        }}
      >
        {typeof children === "function" ? children(state) : children}
      </div>
    )
  }
)

export const ComboboxEmpty = forwardRef<PublicInstance, Props>(
  function ComboboxEmpty(props, ref) {
    const context = useComboboxContext("ComboboxEmpty")
    return context.filteredItems.length === 0 ? <div {...props} ref={ref} /> : null
  }
)

export const ComboboxGroup = forwardRef<PublicInstance, Props>(
  function ComboboxGroup(props, ref) {
    return <div {...props} ref={ref} />
  }
)

export const ComboboxLabel = forwardRef<PublicInstance, Props>(
  function ComboboxLabel(props, ref) {
    return <div {...props} ref={ref} />
  }
)

export const ComboboxSeparator = forwardRef<PublicInstance, Props>(
  function ComboboxSeparator(props, ref) {
    return <div {...props} ref={ref} />
  }
)

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
