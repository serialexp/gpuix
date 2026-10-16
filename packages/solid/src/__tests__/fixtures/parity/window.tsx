// Window methods through the app's renderer: the size and the focus API.
import { createSignal } from "solid-js"
import { useGpuixRequired } from "@gpuix/solid"

export function App() {
  const renderer = useGpuixRequired()
  const size = renderer.getWindowSize?.()
  const [focused, setFocused] = createSignal("unknown")
  return (
    <div style={{ width: 400, height: 300, flexDirection: "column" }}>
      <text>{`Size: ${size?.width}x${size?.height}`}</text>
      <div testId="target" tabIndex={0} style={{ width: 100, height: 30 }}>
        <text>Target</text>
      </div>
      <div
        testId="check"
        style={{ width: 100, height: 30 }}
        onClick={() => {
          renderer.focusNext?.()
          setFocused(String(renderer.getFocusedElementId?.() != null))
        }}
      >
        <text>Check</text>
      </div>
      <text>{`Focused something: ${focused()}`}</text>
    </div>
  )
}
