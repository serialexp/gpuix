import { performance } from "node:perf_hooks"
import { GpuixRenderer } from "../packages/native/index.js"

const rows = Number(process.env.ROWS ?? 10_000)
const iterations = Number(process.env.ITERATIONS ?? 20)
const warmup = Number(process.env.WARMUP ?? 200)
const engine = process.env.ENGINE ?? "Lua 5.4"
const memo = process.env.MEMO === "1"
const batch = process.env.BATCH === "1"
const luax = process.env.LUAX === "1"
const renderer = new GpuixRenderer()
const nodeCount = rows * 4 + 1
const renderRow = luax
  ? `
    return <div key={index} style={row_style}>
      <text style={index_style}>{tostring(index)}</text>
      <text>A representative chat row with nested text</text>
      <text>{is_selected and "selected" or ""}</text>
    </div>`
  : `
    return ui.div {
      key = index,
      style = row_style,
      ui.text { style = index_style, content = tostring(index) },
      ui.text("A representative chat row with nested text"),
      ui.text(is_selected and "selected" or ""),
    }`
const renderRoot = luax
  ? `
    return <div
      onClick={function()
        set_selected(function(value) return value == rows and 1 or value + 1 end)
      end}
      children={children}
    />`
  : `
    return ui.div {
      onClick = function()
        set_selected(function(value) return value == rows and 1 or value + 1 end)
      end,
      children = children,
    }`

const mountStart = performance.now()
const source = `
  local ui = gpuix
  local rows = ${rows}
  local memo = ${memo}
  local batch = ${batch}
  local row_style = ui.style {
    display = "flex",
    flexDirection = "row",
    gap = 8,
    padding = 4,
  }
  local index_style = ui.style { color = "#94a3b8" }
  local row_keys = {}
  for index = 1, rows do row_keys[index] = index end

  local function render_row(index, is_selected)
    ${renderRow}
  end

  return function()
    local selected, set_selected = ui.use_state(1)
    local children
    if batch then
      local dependencies = {}
      for index = 1, rows do
        dependencies[index] = selected == index
      end
      children = ui.memo_batch(row_keys, dependencies, render_row)
    else
      children = {}
      for index = 1, rows do
        local is_selected = selected == index
        if memo then
          children[index] = ui.memo(index, is_selected, function()
            return render_row(index, is_selected)
          end)
        else
          children[index] = render_row(index, is_selected)
        end
      end
    end
    ${renderRoot}
  end
`
if (luax) renderer.loadLuax(source)
else renderer.loadLua(source)
const mountMs = performance.now() - mountStart

for (let index = 0; index < warmup; index++) {
  renderer.dispatchLuaEvent({ elementId: 1, eventType: "click" })
}

const samples: number[] = []
for (let index = 0; index < iterations; index++) {
  const start = performance.now()
  renderer.dispatchLuaEvent({ elementId: 1, eventType: "click" })
  samples.push(performance.now() - start)
}
samples.sort((left, right) => left - right)

console.log(
  `${engine} ${luax ? "LuaX" : "Lua"} direct reconciliation (${rows.toLocaleString()} rows, ${nodeCount.toLocaleString()} nodes, ${batch ? "memo batch" : memo ? "memo" : "full"})`
)
console.log(`compile + mount: ${mountMs.toFixed(2)} ms`)
console.log(`median update: ${samples[Math.floor(samples.length / 2)].toFixed(2)} ms`)
console.log(`p90 update:    ${samples[Math.floor(samples.length * 0.9)].toFixed(2)} ms`)
