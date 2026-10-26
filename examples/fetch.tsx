import React, { useEffect, useRef, useState } from "react"
import { render } from "@gpuix/react"

function App() {
  const [loading, setLoading] = useState(false)
  const [message, setMessage] = useState("Look up your public IPv4 using ipify.")
  const request = useRef<AbortController | null>(null)
  useEffect(() => () => request.current?.abort(), [])

  async function lookup() {
    if (request.current) return
    const controller = new AbortController()
    request.current = controller
    setLoading(true)
    setMessage("Contacting ipify…")
    try {
      const response = await fetch("https://api.ipify.org?format=json", { signal: controller.signal })
      if (!response.ok) throw new Error(`HTTP ${response.status}`)
      const data = await response.json()
      if (typeof data.ip !== "string") throw new Error("Invalid ipify response")
      setMessage(`Public IPv4: ${data.ip}`)
    } catch (error) {
      if (!controller.signal.aborted) setMessage(`Request failed: ${String(error)}`)
    } finally {
      request.current = null
      if (!controller.signal.aborted) setLoading(false)
    }
  }

  return <div style={{ width: "100%", height: "100%", padding: 24, gap: 16,
    display: "flex", flexDirection: "column", background: "#17191e", color: "#eceef2" }}>
    <text style={{ fontSize: 20 }}>HTTP requests</text>
    <div testId="lookup-ip" tabIndex={0} onClick={lookup}
      style={{ padding: 12, borderRadius: 6, background: loading ? "#31343b" : "#3e506b" }}>
      <text>{loading ? "Looking up IPv4…" : "Look up public IPv4"}</text>
    </div>
    <text testId="public-ip-result">{message}</text>
    <text>Open the native inspector’s Network tab to see this request.</text>
  </div>
}

render(<App />, { title: "GPUIX Fetch", width: 620, height: 300 })
