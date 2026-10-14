---
'@gpuix/native': minor
---

Add an experimental embedded Lua 5.4 / LuaJIT runtime. LuaX components run inside Rust and update the retained tree directly, with no JavaScript. Components use React-style hooks, or Solid-style signals with `--!gpuix solid` on the first line. Run an app with the `gpx` binary (`--watch`, `--check`). In a Lua window, Tab moves focus, Enter or Space clicks the focused element, and pointer presses move focus the way a web page does, all natively. React and Solid windows keep their JavaScript focus handling. The bundled `gpuix.icon` component draws any of the 1,512 Phosphor icons in all six weights from SVG data packed into the binary. See "Experimental LuaX runtime" in the README.
