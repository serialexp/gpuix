// Keyed rows reordered, inserted and removed, which moves native elements
// rather than recreating them.
import { useState } from "react"

const button = { width: 80, height: 30 }

export function App() {
  const [rows, setRows] = useState(["a", "b", "c", "d"])
  const [next, setNext] = useState(0)
  return (
    <div style={{ width: 400, height: 400, flexDirection: "column" }}>
      <div style={{ flexDirection: "row", gap: 4 }}>
        <div testId="reverse" style={button} onClick={() => setRows((rows) => [...rows].reverse())}>
          <text>Reverse</text>
        </div>
        <div
          testId="insert"
          style={button}
          onClick={() => {
            setRows((rows) => [rows[0]!, `new${next}`, ...rows.slice(1)])
            setNext((next) => next + 1)
          }}
        >
          <text>Insert</text>
        </div>
        <div testId="remove" style={button} onClick={() => setRows((rows) => rows.slice(0, -1))}>
          <text>Remove</text>
        </div>
      </div>
      {rows.map((row) => (
        <div key={row} testId={`row-${row}`} style={{ height: 24 }}>
          <text>{`Row ${row}`}</text>
        </div>
      ))}
    </div>
  )
}
