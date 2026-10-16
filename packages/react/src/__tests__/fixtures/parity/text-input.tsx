// A controlled input the native editor types into.
import { useState } from "react"
import type { EventPayload } from "@gpuix/native"

export function App() {
  const [text, setText] = useState("")
  return (
    <div style={{ width: 400, height: 100, flexDirection: "column" }}>
      <input
        testId="input"
        value={text}
        placeholder="Type here..."
        style={{ width: 300, height: 40 }}
        onChange={(event: EventPayload) => setText((event.value ?? "").toUpperCase())}
      />
      <text>{`Value: ${text}`}</text>
    </div>
  )
}
