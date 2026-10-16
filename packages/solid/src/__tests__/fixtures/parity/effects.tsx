// Effects, `onSettled`, a zero-delay timer an effect starts, and the
// cleanups that run when a child unmounts.
import { createEffect, createSignal, For, onCleanup, onSettled, Show } from "solid-js"

function Child(props: { onLog: (line: string) => void }) {
  onSettled(() => {
    props.onLog("child settled")
    return () => props.onLog("child settled cleanup")
  })
  createEffect(
    () => props.onLog,
    (onLog) => {
      onLog("child effect")
      return () => onLog("child effect cleanup")
    }
  )
  onCleanup(() => props.onLog("child cleanup"))
  return <text>Child</text>
}

export function App() {
  const [log, setLog] = createSignal<string[]>([])
  const [shown, setShown] = createSignal(true)
  const [timer, setTimer] = createSignal("waiting")
  const onLog = (line: string) => setLog((log) => [...log, line])
  onSettled(() => {
    const id = setTimeout(() => setTimer("fired"), 0)
    return () => clearTimeout(id)
  })
  return (
    <div style={{ width: 400, height: 300, flexDirection: "column" }}>
      <div testId="toggle" style={{ width: 100, height: 30 }} onClick={() => setShown((shown) => !shown)}>
        <text>Toggle</text>
      </div>
      <Show when={shown()}>
        <Child onLog={onLog} />
      </Show>
      <text>{`Timer: ${timer()}`}</text>
      <For each={log()}>{(line) => <text>{line}</text>}</For>
    </div>
  )
}
