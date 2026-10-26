---
"@gpuix/native": minor
---

Add gpx test for declarative native UI scenarios with retrying assertions,
background app launches, screenshots and standalone JSON/HTML reports.
Shared native automation key presses now include key-up events, so keyboard
button activation works in scenarios and existing debug clients.
Suites reuse native windows while recreating the interpreter between cases,
and optionally display live progress in a separate native results window.
The results window supports test and step selection with screenshot evidence;
UI runs automatically capture every executed step.
Discover scenario directories recursively, combine Lua/React/Solid results in
one independent native viewer, and use the Comet three-column test-runner UI
with filtering, keyboard navigation, reruns and snapshot/tree evidence.
