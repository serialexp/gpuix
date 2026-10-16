import {
  children,
  createComponent,
  createContext,
  createRenderEffect,
  createSignal,
  For,
  useContext,
  type Accessor,
  type Component,
} from "solid-js"
import type { JSX } from "../jsx-runtime.js"
import type { EventPayload } from "@gpuix/native"
import type { MotionProps } from "@gpuix/native/host"
import { isHostElement, setHostProperty } from "../host.js"

export interface PresenceContextValue {
  isPresent: Accessor<boolean>
  initial: false | undefined
  safeToRemove(): void
}

// Null outside AnimatePresence: Solid 2.0 throws for a context with no default.
export const PresenceContext = createContext<PresenceContextValue | null>(null)

export function usePresence(): [Accessor<boolean>, () => void] {
  const context = useContext(PresenceContext)
  return context
    ? [context.isPresent, context.safeToRemove]
    : [() => true, () => {}]
}

export function useIsPresent(): Accessor<boolean> {
  return usePresence()[0]
}

export interface AnimatePresenceProps {
  children?: JSX.Element
  initial?: boolean
  onExitComplete?: () => void
}

type NativeMotionProps = MotionProps & {
  generation?: number
  isExit?: boolean
}

// Written while the children update, inside the render effect.
const INTERNAL = { ownedWrite: true } as const

/** `For` with its keyed child shape, which createComponent can't pick out of
 *  the overloads. */
const KeyedFor = For as Component<{
  each: readonly JSX.Element[]
  children: (item: JSX.Element, index: Accessor<number>) => JSX.Element
}>

function normalize(value: JSX.Element): JSX.Element[] {
  if (value == null || value === true || value === false) return []
  return Array.isArray(value) ? value.flatMap(normalize) : [value]
}

export function AnimatePresence(props: AnimatePresenceProps): JSX.Element {
  const resolved = children(() => props.children)
  // The state lives in plain variables, and the signals publish it: Solid 2.0
  // shows a write only after the next flush, and a removal right after an
  // update has to see that update.
  let rendered: JSX.Element[] = []
  const exiting = new Set<JSX.Element>()
  // Children of the first render, while `initial={false}` holds for them.
  const initialItems = new Set<JSX.Element>()
  // What an exit replaced on a host element, to put back if it returns.
  const beforeExit = new Map<JSX.Element, {
    motion: NativeMotionProps
    onMotionComplete?: (event: EventPayload) => void
  }>()
  let first = true
  const [list, setList] = createSignal<JSX.Element[]>([], INTERNAL)
  const [exitingNow, setExitingNow] = createSignal<ReadonlySet<JSX.Element>>(new Set(), INTERNAL)
  const publish = () => {
    setList([...rendered])
    setExitingNow(new Set(exiting))
  }

  const remove = (item: JSX.Element) => {
    if (!exiting.delete(item)) return
    beforeExit.delete(item)
    rendered = rendered.filter((candidate) => candidate !== item)
    publish()
    if (exiting.size === 0) props.onExitComplete?.()
  }

  const startExit = (item: JSX.Element) => {
    initialItems.delete(item)
    const motion = isHostElement(item)
      ? item.props.get("motion") as NativeMotionProps | undefined
      : undefined
    if (!isHostElement(item) || !motion?.exit) {
      remove(item)
      return
    }
    const previous = item.props.get("onMotionComplete") as
      | ((event: EventPayload) => void)
      | undefined
    beforeExit.set(item, { motion, onMotionComplete: previous })
    setHostProperty(item, "motion", {
      ...motion,
      generation: (motion.generation ?? 0) + 1,
      isExit: true,
      animate: motion.exit,
    }, motion)
    setHostProperty(
      item,
      "onMotionComplete",
      (event: EventPayload) => {
        const active = item.props.get("motion") as NativeMotionProps | undefined
        if (
          active?.generation !== undefined &&
          event.motionGeneration !== active.generation
        ) return
        previous?.(event)
        remove(item)
      },
      previous
    )
  }

  const cancelExit = (item: JSX.Element) => {
    const saved = beforeExit.get(item)
    beforeExit.delete(item)
    if (!saved || !isHostElement(item)) return
    const motion = item.props.get("motion") as NativeMotionProps | undefined
    setHostProperty(item, "motion", {
      ...saved.motion,
      generation: (motion?.generation ?? 0) + 1,
      isExit: false,
    }, motion)
    setHostProperty(
      item,
      "onMotionComplete",
      saved.onMotionComplete,
      item.props.get("onMotionComplete") as ((event: EventPayload) => void) | undefined
    )
  }

  createRenderEffect(
    () => normalize(resolved()),
    (current) => {
      // Gone from the children, in the order they were rendered; those not
      // already exiting start now.
      const gone = rendered.filter((item) => !current.includes(item))
      const leaving = gone.filter((item) => !exiting.has(item))
      for (const item of current) {
        if (exiting.delete(item)) cancelExit(item)
        if (first && props.initial === false) {
          initialItems.add(item)
          if (isHostElement(item)) {
            const motion = item.props.get("motion") as MotionProps | undefined
            if (motion) setHostProperty(item, "motion", { ...motion, initial: false }, motion)
          }
        }
      }
      first = false
      for (const item of leaving) exiting.add(item)
      rendered = [...current, ...gone]
      publish()
      for (const item of leaving) startExit(item)
    }
  )

  return createComponent(KeyedFor, {
    get each() { return list() },
    children: (item: JSX.Element) => createComponent(PresenceContext, {
      value: {
        isPresent: () => !exitingNow().has(item),
        initial: initialItems.has(item) ? false : undefined,
        safeToRemove: () => remove(item),
      },
      get children() { return item },
    }),
  })
}
