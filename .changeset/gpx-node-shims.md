---
'@gpuix/native': minor
'@gpuix/react': patch
'@gpuix/solid': patch
---

`gpx` JavaScript apps get a read-only `process` (`env`, `argv`, `platform`), `import.meta.url` and `import.meta.main`, and can import their own files as text with `import x from "./file.md" with { type: "text" }`. `@gpuix/react` installs its uncaught-error handlers only where `process.on` exists, `@gpuix/react` and `@gpuix/solid` exit on window close only where `process.exit` exists, and `applyMacCpuThrottleFromEnv` says it needs Node or Bun instead of failing with "not a function".
