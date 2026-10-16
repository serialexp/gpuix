// The bundled Select: it opens in a floating layer, moves its highlight
// with the keyboard, and closes on a choice.
import { createSignal } from "solid-js"
import { Select, SelectContent, SelectItem, SelectTrigger } from "@gpuix/solid/select"

export function App() {
  const [value, setValue] = createSignal("apple")
  const item = (state: { highlighted: boolean }) => ({
    height: 28,
    backgroundColor: state.highlighted ? "#335577" : "#222222",
  })
  return (
    <div style={{ width: 400, height: 300, flexDirection: "column", gap: 8 }}>
      <Select value={value()} onValueChange={setValue}>
        <SelectTrigger testId="trigger" style={{ width: 160, height: 32, backgroundColor: "#333333" }}>
          <text>{`Fruit: ${value()}`}</text>
        </SelectTrigger>
        <SelectContent style={{ width: 160, backgroundColor: "#111111" }}>
          <SelectItem value="apple" testId="apple" style={item}><text>Apple</text></SelectItem>
          <SelectItem value="banana" testId="banana" style={item}><text>Banana</text></SelectItem>
          <SelectItem value="cherry" testId="cherry" style={item}><text>Cherry</text></SelectItem>
        </SelectContent>
      </Select>
      <text>{`Chosen: ${value()}`}</text>
    </div>
  )
}
