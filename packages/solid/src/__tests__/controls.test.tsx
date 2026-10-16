import { createSignal } from "solid-js"
import { describe, expect, it } from "bun:test"
import {
  Button,
  Combobox,
  ComboboxContent,
  Dialog,
  DialogBackdrop,
  DialogClose,
  DialogPopup,
  DialogPortal,
  DialogTitle,
  DialogTrigger,
  ComboboxInput,
  ComboboxItem,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "../index.js"
import { createTestRoot, hasNativeTestRenderer } from "../testing.js"
import type { HostElement } from "../host.js"

describe.skipIf(!hasNativeTestRenderer)("Solid controls", () => {
  it("selects with keyboard and click", () => {
    const [value, setValue] = createSignal("a")
    const app = createTestRoot()
    app.render(() => (
      <Select value={value()} onValueChange={setValue}>
        <SelectTrigger testId="trigger" style={{ width: 120, height: 32, backgroundColor: "#333333" }}>
          <text>open</text>
        </SelectTrigger>
        <SelectContent style={{ width: 120, backgroundColor: "#222222" }}>
          <SelectItem value="a"><text>A</text></SelectItem>
          <SelectItem value="b" testId="b" style={{ height: 28 }}><text>B</text></SelectItem>
        </SelectContent>
      </Select>
    ))
    const trigger = app.renderer.findByTestId("trigger")!
    app.renderer.nativeSimulateKeyDown(trigger.id, "down")
    app.renderer.simulateKeystrokes("down enter")
    expect(value()).toBe("b")

    app.renderer.nativeSimulateClick(60, 16)
    const item = app.renderer.findByTestId("b")!
    const bounds = app.renderer.getElementBounds(item.id)!
    app.renderer.nativeSimulateClick(bounds.x + 10, bounds.y + 10)
    expect(value()).toBe("b")
  })

  it("highlights the selected item when it opens", () => {
    const app = createTestRoot()
    const highlight = (state: { highlighted: boolean }) => ({
      height: 28,
      backgroundColor: state.highlighted ? "#ff0000" : "#000000",
    })
    app.render(() => (
      <Select defaultValue="b">
        <SelectTrigger testId="trigger" style={{ width: 120, height: 32 }}><text>open</text></SelectTrigger>
        <SelectContent style={{ width: 120 }}>
          <SelectItem value="a" testId="a" style={highlight}><text>A</text></SelectItem>
          <SelectItem value="b" testId="b" style={highlight}><text>B</text></SelectItem>
        </SelectContent>
      </Select>
    ))
    const colors = () => ["a", "b"].map((id) => app.renderer.findByTestId(id)?.style.backgroundColor)
    app.renderer.nativeSimulateKeyDown(app.renderer.findByTestId("trigger")!.id, "down")
    expect(colors()).toEqual(["#000000", "#ff0000"])
    app.renderer.simulateKeystrokes("down")
    expect(colors()).toEqual(["#ff0000", "#000000"])
  })

  it("auto-highlights the first enabled match of what was typed", () => {
    const [value, setValue] = createSignal<string | string[] | null>(null)
    const app = createTestRoot()
    app.render(() => (
      <Combobox items={["beta", "alpha", "alps"]} value={value()} onValueChange={setValue} autoHighlight defaultOpen>
        <ComboboxInput testId="input" style={{ width: 180, height: 32 }} />
        <ComboboxContent>
          <ComboboxItem value="beta" disabled><text>beta</text></ComboboxItem>
          <ComboboxItem value="alpha"><text>alpha</text></ComboboxItem>
          <ComboboxItem value="alps"><text>alps</text></ComboboxItem>
        </ComboboxContent>
      </Combobox>
    ))
    const input = app.renderer.findByTestId("input")!
    app.renderer.nativeSimulateKeystrokes(input.id, "a l")
    app.renderer.nativeSimulateKeystrokes(input.id, "enter")
    expect(value()).toBe("alpha")
  })

  it("runs a forwarded handler and the asChild element's own", () => {
    const log: string[] = []
    const handlers = { enter: () => log.push("part") }
    const app = createTestRoot()
    app.render(() => (
      <Button asChild onMouseEnter={handlers.enter}>
        <div testId="target" onMouseEnter={() => log.push("own")} style={{ width: 80, height: 30 }} />
      </Button>
    ))
    const bounds = app.renderer.getElementBounds(app.renderer.findByTestId("target")!.id)!
    app.renderer.nativeSimulateMouseMove(bounds.x + 10, bounds.y + 10)
    expect(log).toEqual(["own", "part"])
  })

  it("keeps a part's bare children live", () => {
    const [label, setLabel] = createSignal("first")
    const app = createTestRoot()
    app.render(() => <Button style={{ width: 80, height: 30 }}>{label()}</Button>)
    expect(app.renderer.getAllText()).toEqual(["first"])
    app.flushSync(() => setLabel("second"))
    expect(app.renderer.getAllText()).toEqual(["second"])
  })

  it("keeps a root's style live after mount", () => {
    const [width, setWidth] = createSignal(100)
    const app = createTestRoot()
    app.render(() => (
      <Select testId="root" defaultValue="a" style={{ width: width() }}>
        <SelectTrigger><text>open</text></SelectTrigger>
      </Select>
    ))
    app.flushSync(() => setWidth(200))
    expect(app.renderer.findByTestId("root")?.style.width).toBe(200)
  })

  it("keeps forwarded props live after mount", () => {
    const [id, setId] = createSignal("first")
    const app = createTestRoot()
    app.render(() => (
      <Tooltip>
        <TooltipTrigger testId={id()} style={{ width: 100, height: 32 }}><text>hover</text></TooltipTrigger>
        <TooltipContent><text>help</text></TooltipContent>
      </Tooltip>
    ))
    expect(app.renderer.findByTestId("first")).toBeDefined()
    app.flushSync(() => setId("second"))
    expect(app.renderer.findByTestId("first")).toBeUndefined()
    expect(app.renderer.findByTestId("second")).toBeDefined()
  })

  it("filters and submits a combobox item", () => {
    const [value, setValue] = createSignal<string | string[] | null>(null)
    const app = createTestRoot()
    app.render(() => (
      <Combobox items={["alpha", "beta"]} value={value()} onValueChange={setValue} defaultOpen>
        <ComboboxInput testId="input" style={{ width: 180, height: 32 }} />
        <ComboboxContent>
          <ComboboxItem value="alpha"><text>alpha</text></ComboboxItem>
          <ComboboxItem value="beta"><text>beta</text></ComboboxItem>
        </ComboboxContent>
      </Combobox>
    ))
    const input = app.renderer.findByTestId("input")!
    app.renderer.nativeSimulateKeyDown(input.id, "down")
    app.renderer.nativeSimulateKeystrokes(input.id, "enter")
    expect(value()).toBe("alpha")
  })

  it("opens tooltip through a real hover event and closes on click", () => {
    const app = createTestRoot()
    app.render(() => (
      <Tooltip>
        <TooltipTrigger testId="tip-trigger" style={{ width: 100, height: 32, backgroundColor: "#333333" }}>
          <text>hover</text>
        </TooltipTrigger>
        <TooltipContent><text>help</text></TooltipContent>
      </Tooltip>
    ))
    const trigger = app.renderer.findByTestId("tip-trigger")!
    const bounds = app.renderer.getElementBounds(trigger.id)!
    app.renderer.nativeSimulateMouseMove(bounds.x + 10, bounds.y + 10)
    expect(app.renderer.getAllText()).toContain("help")
    app.renderer.nativeSimulateClick(bounds.x + 10, bounds.y + 10)
    expect(app.renderer.getAllText()).not.toContain("help")
  })

  it("moves focus with Tab through asChild triggers and keeps child handlers", () => {
    const keys: string[] = []
    const app = createTestRoot()
    app.render(() => (
      <div style={{ display: "flex", width: 500, height: 300, gap: 40 }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <div autoFocus tabIndex={0} testId="before" style={{ width: 80, height: 24 }} />
          <Select defaultValue="a">
            <SelectTrigger asChild>
              <div
                testId="trigger"
                onKeyDown={(event) => keys.push(`child:${event.key}`)}
                style={{ width: 120, height: 32, backgroundColor: "#333333" }}
              >
                <text>open</text>
              </div>
            </SelectTrigger>
            <SelectContent testId="content" style={{ width: 120, backgroundColor: "#222222" }}>
              <SelectItem value="a"><text>A</text></SelectItem>
              <SelectItem value="b"><text>B</text></SelectItem>
            </SelectContent>
          </Select>
          <Tooltip>
            <TooltipTrigger asChild>
              <div testId="tip" style={{ width: 80, height: 24 }}><text>tip</text></div>
            </TooltipTrigger>
            <TooltipContent><text>help</text></TooltipContent>
          </Tooltip>
        </div>
        <input testId="outside" style={{ width: 120, height: 28 }} />
      </div>
    ))
    const focused = () => app.renderer.getElement(app.renderer.getFocusedElementId()!)?.testId
    const steps: string[] = []
    const step = (label: string) => steps.push(`${label}: focus=${focused()} text=${app.renderer.getAllText().join("|")}`)

    app.renderer.simulateKeystrokes("tab")
    step("tab")
    app.renderer.simulateKeystrokes("enter")
    step("enter")
    app.renderer.simulateKeystrokes("tab")
    step("tab in popup")
    app.renderer.simulateKeystrokes("escape")
    step("escape")
    app.renderer.simulateKeystrokes("tab")
    step("tab")
    const trigger = app.renderer.getElementBounds(app.renderer.findByTestId("trigger")!.id)!
    app.renderer.nativeSimulateClick(trigger.x + 10, trigger.y + 10)
    const outside = app.renderer.getElementBounds(app.renderer.findByTestId("outside")!.id)!
    app.renderer.nativeSimulateClick(outside.x + 10, outside.y + 10)
    step("click outside")

    expect(steps).toEqual([
      "tab: focus=trigger text=open|tip",
      "enter: focus=content text=open|A|B|tip",
      "tab in popup: focus=content text=open|A|B|tip",
      "escape: focus=trigger text=open|tip",
      "tab: focus=tip text=open|tip|help",
      "click outside: focus=outside text=open|tip",
    ])
    // The child's own onKeyDown still runs after asChild merges the trigger's.
    expect(keys).toEqual(["child:enter", "child:tab"])
  })

  it("runs a modal Dialog: trap Tab, Escape closes the nested Select first", () => {
    const app = createTestRoot()
    app.render(() => (
      <div style={{ display: "flex", flexDirection: "column", width: 600, height: 400, gap: 8 }}>
        <Dialog>
          <DialogTrigger testId="open" style={{ width: 80, height: 24 }}><text>Open</text></DialogTrigger>
          <DialogPortal>
            <DialogBackdrop style={{ backgroundColor: "#00000080" }} />
            <DialogPopup testId="popup" style={{ width: 300, padding: 16, gap: 8, backgroundColor: "#1e293b" }}>
              <DialogTitle><text>Settings</text></DialogTitle>
              <Select defaultValue="a">
                <SelectTrigger testId="select" style={{ width: 120, height: 32, backgroundColor: "#333333" }}>
                  <text>pick</text>
                </SelectTrigger>
                <SelectContent style={{ width: 120, backgroundColor: "#222222" }}>
                  <SelectItem value="a"><text>A</text></SelectItem>
                </SelectContent>
              </Select>
              <DialogClose testId="close" style={{ width: 60, height: 24 }}><text>Close</text></DialogClose>
            </DialogPopup>
          </DialogPortal>
        </Dialog>
      </div>
    ))
    const focused = () => app.renderer.getElement(app.renderer.getFocusedElementId()!)?.testId
    const steps: string[] = []
    const press = (keys: string) => {
      app.renderer.simulateKeystrokes(keys)
      steps.push(`${keys}: focus=${focused()} text=${app.renderer.getAllText().join("|")}`)
    }
    app.renderer.focusElement(app.renderer.findByTestId("open")!.id)
    press("enter")
    press("tab")
    press("tab")
    press("tab")
    press("down")
    press("escape")
    press("escape")
    press("enter")
    const popup = app.renderer.getElementBounds(app.renderer.findByTestId("popup")!.id)!
    app.renderer.nativeSimulateClick(popup.x + popup.width - 4, popup.y + 4)
    steps.push(`press popup: text=${app.renderer.getAllText().join("|")}`)
    app.renderer.nativeSimulateClick(4, 390)
    steps.push(`press backdrop: focus=${focused()} text=${app.renderer.getAllText().join("|")}`)
    expect(steps).toEqual([
      "enter: focus=popup text=Open|Settings|pick|Close",
      "tab: focus=select text=Open|Settings|pick|Close",
      "tab: focus=close text=Open|Settings|pick|Close",
      "tab: focus=select text=Open|Settings|pick|Close",
      "down: focus=undefined text=Open|Settings|pick|A|Close",
      "escape: focus=select text=Open|Settings|pick|Close",
      "escape: focus=open text=Open",
      "enter: focus=popup text=Open|Settings|pick|Close",
      "press popup: text=Open|Settings|pick|Close",
      "press backdrop: focus=open text=Open",
    ])
  })

  it("focuses initialFocus on open and returns focus without a Trigger", () => {
    const [open, setOpen] = createSignal(false)
    let field!: HostElement
    const app = createTestRoot()
    app.render(() => (
      <div style={{ display: "flex", flexDirection: "column", width: 600, height: 400, gap: 8 }}>
        <div
          autoFocus
          tabIndex={0}
          testId="launcher"
          onKeyDown={(event) => {
            if (event.key === "enter") setOpen(true)
          }}
          style={{ width: 80, height: 24 }}
        />
        <Dialog open={open()} onOpenChange={setOpen}>
          <DialogPortal>
            <DialogPopup initialFocus={() => field} style={{ width: 300, padding: 16, backgroundColor: "#1e293b" }}>
              <div tabIndex={0} testId="first" style={{ width: 60, height: 24 }} />
              <input ref={field} testId="field" style={{ width: 160, height: 28 }} />
            </DialogPopup>
          </DialogPortal>
        </Dialog>
      </div>
    ))
    const focused = () => app.renderer.getElement(app.renderer.getFocusedElementId()!)?.testId
    const steps: string[] = []
    app.renderer.simulateKeystrokes("enter")
    steps.push(`open: ${focused()}`)
    app.renderer.simulateKeystrokes("escape")
    steps.push(`close: ${focused()}`)
    expect(steps).toEqual(["open: field", "close: launcher"])
  })

  it("activates a Button on Enter down and Space up, and skips it while disabled", () => {
    const log: string[] = []
    const [disabled, setDisabled] = createSignal(false)
    const app = createTestRoot()
    app.render(() => (
      <div style={{ display: "flex", gap: 8 }}>
        <div autoFocus tabIndex={0} testId="start" style={{ width: 20, height: 20 }} />
        <Button testId="save" disabled={disabled()} onClick={() => log.push("save")} style={{ width: 60, height: 24 }}>
          <text>Save</text>
        </Button>
        <div tabIndex={0} testId="end" style={{ width: 20, height: 20 }} />
      </div>
    ))
    const focused = () => app.renderer.getElement(app.renderer.getFocusedElementId()!)?.testId
    app.renderer.simulateKeystrokes("tab")
    log.push(`focus ${focused()}`)
    app.renderer.simulateKeystrokes("enter")
    app.renderer.nativeSimulateKeyDown(app.renderer.getFocusedElementId()!, "space")
    log.push("space down")
    app.renderer.nativeSimulateKeyUp(app.renderer.getFocusedElementId()!, "space")
    app.flushSync(() => setDisabled(true))
    app.renderer.focusElement(app.renderer.findByTestId("start")!.id)
    app.renderer.simulateKeystrokes("tab")
    log.push(`focus ${focused()}`)
    expect(log).toEqual(["focus save", "save", "space down", "save", "focus end"])
  })
})
