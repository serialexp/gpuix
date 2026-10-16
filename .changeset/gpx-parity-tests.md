---
'@gpuix/native': minor
---

`TestRenderer.loadJs(entry)` runs a JavaScript app in the test window as `gpx` runs it, so a test can hold the app to what it does under Node; `runJsTimers()` runs its timers, and the window's events go to it. It needs a native build with the `quickjs` feature (`bun run build:quickjs`), which `hasQuickJsTestRenderer` reports; published builds don't include it.

A `gpx` app's first render now sees the size its window opens at, as under Node, rather than an 800x600 fallback until `useWindowSize` polls again. After a reload, that's the window's current size.
