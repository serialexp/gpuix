---
"@gpuix/native": minor
---

Replace source-build packaging with a self-contained native packer. Installed
gpx binaries package application archives into copies of their own runner without
Bun, Rust or a GPUIX checkout. Preserve native macOS bundles and Debian packages,
with embedded default icons, safe output replacement and signed Mach-O archives.
