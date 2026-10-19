---
"@gpuix/native": patch
---

Add an opt-in authenticated loopback HTTP testing API to `gpx --dev-server`,
shared by Lua and QuickJS apps, with source watching, window discovery,
retained-tree inspection (including optional styles, events and custom props),
window-targeted PNG screenshots, native clicks, and keyboard dispatch.

Support subtree exports by node ID or unique testId and opaque revision tokens
for conditional polling without resending unchanged sections.
