// Keyed rows reordered, inserted and removed, which moves native elements
// rather than recreating them.
import { createSignal, For } from "solid-js"

const button = { width: 80, height: 30 }

export function App() {
  const [rows, setRows] = createSignal(["a", "b", "c", "d"])
  let next = 0
  return (
    <div style={{ width: 400, height: 400, flexDirection: "column" }}>
      <div style={{ flexDirection: "row", gap: 4 }}>
        <div testId="reverse" style={button} onClick={() => setRows((rows) => [...rows].reverse())}>
          <text>Reverse</text>
        </div>
        <div
          testId="insert"
          style={button}
          onClick={() => setRows((rows) => [rows[0]!, `new${next++}`, ...rows.slice(1)])}
        >
          <text>Insert</text>
        </div>
        <div testId="remove" style={button} onClick={() => setRows((rows) => rows.slice(0, -1))}>
          <text>Remove</text>
        </div>
      </div>
      <For each={rows()}>
        {(row) => (
          <div testId={`row-${row}`} style={{ height: 24 }}>
            <text>{`Row ${row}`}</text>
          </div>
        )}
      </For>
    </div>
  )
}
