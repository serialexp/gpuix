// How quickjs-ng's `Map` scales with the key types SolidJS uses: small
// integers (`For` items) and objects (nodes in its array reconciler), with
// strings to compare. A good hash keeps the time per operation flat as the
// map grows. Run it with quickjs-ng's `qjs`, built with and without
// `rquickjs-sys-0.14.0-map-hash-mix.diff`:
//
//     qjs map-hash-repro.js
//
// Without the fix, integer and object keys take several times longer per
// operation than strings (see docs/design/quickjs-phase0-results.md).

function nanosecondsPerOperation(kind, size) {
  const keys = []
  for (let i = 1; i <= size; i++) {
    keys.push(kind === "int" ? i : kind === "string" ? "k" + i : { i })
  }
  const start = Date.now()
  let rounds = 0
  while (Date.now() - start < 200) {
    const map = new Map()
    for (let i = 0; i < size; i++) map.set(keys[i], i)
    for (let i = 0; i < size; i++) map.get(keys[i])
    rounds++
  }
  return ((Date.now() - start) * 1e6) / (rounds * size * 2)
}

console.log("keys         size      ns per op")
for (const kind of ["int", "object", "string"]) {
  for (const size of [1_000, 10_000, 100_000]) {
    const ns = nanosecondsPerOperation(kind, size).toFixed(1)
    console.log(`${kind.padEnd(8)} ${String(size).padStart(8)} ${ns.padStart(14)}`)
  }
}
