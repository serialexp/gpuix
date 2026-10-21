export interface TrayItem {
  label: string
  onClick: () => void
  checked?: boolean
  enabled?: boolean
}

export interface TrayOptions {
  icon: string
  tooltip?: string
  menu: Array<TrayItem | { separator: true }>
}

export interface TrayHandle { readonly id: number }

interface AppHost {
  allocateTrayId(): number
  trayCommand(json: string): void
  setTrayHandler(handler: (action: number) => void): void
}

const callbacks = new Map<number, () => void>()
const handles = new WeakMap<TrayHandle, number[]>()

function host(): AppHost {
  const value = (globalThis as unknown as { __gpuix?: AppHost }).__gpuix
  if (!value?.trayCommand) throw new Error("Application trays require the macOS gpx embedded runtime")
  return value
}

function replace(tray: TrayHandle, options: TrayOptions): void {
  const bridge = host()
  const next = new Map<number, () => void>()
  const menu = options.menu.map(item => {
    if ("separator" in item) {
      if (item.separator !== true) throw new TypeError("A separator must be true")
      return { separator: true }
    }
    if (typeof item.onClick !== "function") throw new TypeError("Tray items require onClick")
    const action = bridge.allocateTrayId()
    next.set(action, item.onClick)
    return { label: item.label, action, checked: item.checked ?? false, enabled: item.enabled ?? true }
  })
  bridge.trayCommand(JSON.stringify({ method: "set", spec: { id: tray.id, icon: options.icon, tooltip: options.tooltip ?? "", menu } }))
  for (const action of handles.get(tray) ?? []) callbacks.delete(action)
  for (const [action, callback] of next) callbacks.set(action, callback)
  handles.set(tray, [...next.keys()])
}

export function createTray(options: TrayOptions): TrayHandle {
  const bridge = host()
  bridge.setTrayHandler(action => callbacks.get(action)?.())
  const tray = Object.freeze({ id: bridge.allocateTrayId() })
  replace(tray, options)
  return tray
}

export function updateTray(tray: TrayHandle, options: TrayOptions): void {
  if (!handles.has(tray)) throw new Error("Tray has been removed or belongs to another runtime")
  replace(tray, options)
}

export function removeTray(tray: TrayHandle): void {
  const actions = handles.get(tray)
  if (!actions) return
  host().trayCommand(JSON.stringify({ method: "remove", id: tray.id }))
  for (const action of actions) callbacks.delete(action)
  handles.delete(tray)
}

export function showWindow(): void { host().trayCommand('{"method":"showWindow"}') }
export function hideWindow(): void { host().trayCommand('{"method":"hideWindow"}') }
export function quit(): void { host().trayCommand('{"method":"quit"}') }
