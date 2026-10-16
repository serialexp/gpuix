import { createRenderEffect, merge, omit, useContext } from "solid-js"
import type { JSX } from "../jsx-runtime.js"
import type { MotionProps, SolidHostProps } from "../jsx-runtime.js"
import { createElement, spread } from "../universal.js"
import { PresenceContext } from "./animate-presence.js"

export interface MotionDivProps extends SolidHostProps, MotionProps {}

function MotionDiv(props: MotionDivProps): JSX.Element {
  const hostProps = omit(props, "initial", "animate", "exit", "transition")
  const presence = useContext(PresenceContext)
  createRenderEffect(
    () => presence !== null && !presence.isPresent() && !props.exit,
    (removable) => {
      if (removable) presence!.safeToRemove()
    }
  )
  const element = createElement("div")
  let generation = 0
  let targetKey: string | undefined
  spread(element, merge(hostProps, {
    get motion() {
      const isExit = Boolean(presence && !presence.isPresent() && props.exit)
      const initial = presence?.initial === false ? false : props.initial
      const animate = isExit ? props.exit! : props.animate
      const nextKey = JSON.stringify([isExit, initial, animate, props.transition])
      if (nextKey !== targetKey) {
        targetKey = nextKey
        generation += 1
      }
      return {
        generation,
        isExit,
        initial,
        animate,
        exit: props.exit,
        transition: props.transition,
      }
    },
    onMotionComplete(event: Parameters<NonNullable<MotionDivProps["onMotionComplete"]>>[0]) {
      if (event.motionGeneration !== generation) return
      props.onMotionComplete?.(event)
      if (presence && !presence.isPresent()) presence.safeToRemove()
    },
  }))
  return element as never
}

export const motion = { div: MotionDiv } as const
