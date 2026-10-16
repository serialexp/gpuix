// A click updates a signal, and the update restyles what it touched.
import { createSignal } from "solid-js"

export function App() {
  const [count, setCount] = createSignal(0)
  return (
    <div style={{ width: 320, height: 200, flexDirection: "column", gap: 8 }}>
      <div
        testId="increment"
        style={{
          width: 120,
          height: 40,
          backgroundColor: count() % 2 === 0 ? "#224466" : "#664422",
        }}
        onClick={() => setCount((value) => value + 1)}
      >
        <text>Increment</text>
      </div>
      <text>{`Count: ${count()}`}</text>
    </div>
  )
}
