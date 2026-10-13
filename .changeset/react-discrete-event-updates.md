---
'@gpuix/react': minor
'@gpuix/native': patch
---

React state updates now get the priority React DOM gives them. An update made in a click, key, focus, change or other discrete event handler commits in a microtask, before the event's turn ends, instead of on the next `setImmediate`. Updates from mouse movement and scrolling still go through React's scheduler, and so do updates made outside an event. Code that read the tree straight after a click and expected the old state will see the new one sooner. `RendererRootHandlers.schedule` in `@gpuix/native/host` now also receives the event.
