// A click updates state, and the update restyles what it touched.
import { useState } from "react"

export function App() {
  const [count, setCount] = useState(0)
  return (
    <div style={{ width: 320, height: 200, flexDirection: "column", gap: 8 }}>
      <div
        testId="increment"
        style={{
          width: 120,
          height: 40,
          backgroundColor: count % 2 === 0 ? "#224466" : "#664422",
        }}
        onClick={() => setCount((value) => value + 1)}
      >
        <text>Increment</text>
      </div>
      <text>{`Count: ${count}`}</text>
    </div>
  )
}
