---
"@gpuix/native": minor
---

Add asynchronous gpuix.fetch and cancel_fetch to standalone Lua/LuaX apps,
using the existing HTTP client and application event loop. Support binary-safe
request/response bodies, headers, JSON decoding, timeouts and bounded buffering.
