---
"@gpuix/native": minor
---

Expose manifest-driven application packaging through `gpx package`, including
binary, macOS app and Debian package formats. Keep application runner builds
separate from the running CLI, and resolve package outputs from the caller's
working directory. Packaging currently requires the source checkout, Bun and Rust.
