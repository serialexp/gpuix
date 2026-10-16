// Layout and passive effects, a zero-delay timer an effect starts, and the
// cleanups that run when a child unmounts.
import { useEffect, useLayoutEffect, useState } from "react"

function Child({ onLog }: { onLog: (line: string) => void }) {
  useLayoutEffect(() => {
    onLog("child layout effect")
    return () => onLog("child layout cleanup")
  }, [onLog])
  useEffect(() => {
    onLog("child effect")
    return () => onLog("child cleanup")
  }, [onLog])
  return <text>Child</text>
}

export function App() {
  const [log, setLog] = useState<string[]>([])
  const [shown, setShown] = useState(true)
  const [timer, setTimer] = useState("waiting")
  const [onLog] = useState(() => (line: string) => setLog((log) => [...log, line]))
  useEffect(() => {
    const id = setTimeout(() => setTimer("fired"), 0)
    return () => clearTimeout(id)
  }, [])
  return (
    <div style={{ width: 400, height: 300, flexDirection: "column" }}>
      <div testId="toggle" style={{ width: 100, height: 30 }} onClick={() => setShown((shown) => !shown)}>
        <text>Toggle</text>
      </div>
      {shown ? <Child onLog={onLog} /> : null}
      <text>{`Timer: ${timer}`}</text>
      {log.map((line, index) => (
        <text key={index}>{line}</text>
      ))}
    </div>
  )
}
