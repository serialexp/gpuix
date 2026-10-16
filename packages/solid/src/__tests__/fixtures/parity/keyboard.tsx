// Focus that moves with Tab, `autoFocus`, and keys reaching the focused
// element and the window.
import { createSignal } from "solid-js"
import type { EventPayload } from "@gpuix/native"

export const options = { tabNavigation: true }

export function App() {
  const [keys, setKeys] = createSignal<string[]>([])
  const [focused, setFocused] = createSignal("none")
  const box = (name: string, autoFocus = false) => (
    <div
      testId={name}
      tabIndex={0}
      autoFocus={autoFocus}
      style={{ width: 100, height: 30 }}
      onFocus={() => setFocused(name)}
      onKeyDown={(event: EventPayload) => setKeys((keys) => [...keys, `${name}:${event.key}`])}
    >
      <text>{name}</text>
    </div>
  )
  return (
    <div style={{ width: 400, height: 300, flexDirection: "column" }}>
      {box("first", true)}
      {box("second")}
      <text>{`Focused: ${focused()}`}</text>
      <text>{`Keys: ${keys().join(" ")}`}</text>
    </div>
  )
}
