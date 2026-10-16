---
'@gpuix/native': minor
---

`gpx` JavaScript apps can use the renderer's window methods: focus and Tab navigation, the text selection, scrolling, images, fonts, window controls, the debug frame overlay and `promptForPaths`. What an app asks of its window before it opens, such as focusing an element from an effect, happens once it has. `gc()` runs a full garbage collection, so `idle-gc` collects in `gpx` too.
