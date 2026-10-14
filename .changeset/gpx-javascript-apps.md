---
'@gpuix/native': minor
---

Add `gpx`, one binary that runs Lua and React apps without Node or Bun. The entry file's extension picks the engine: `.lua` and `.luax` run in the embedded Lua runtime, `.tsx`, `.ts`, `.mts`, `.jsx`, `.js` and `.mjs` in an embedded QuickJS engine, with TypeScript and JSX compiled on load. Flags: `--check`, `--watch`, `--title`, `--width` and `--height`. JavaScript apps can import `react` and `@gpuix/react`; see "Experimental embedded JavaScript runtime" in the README.
