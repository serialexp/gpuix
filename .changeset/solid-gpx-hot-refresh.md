---
"@gpuix/native": patch
---

Enable Solid component hot refresh under `gpx --watch`, preserving unaffected
parent and sibling state while replacing edited components. Expose per-module
hot data and disposal, retain the mounted UI on compile errors, and compose
refresh source maps with the Solid JSX compiler's maps.
