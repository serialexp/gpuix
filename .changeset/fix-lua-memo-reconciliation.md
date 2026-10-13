---
'@gpuix/native': patch
---

Keep Lua apps rendering when a memoized subtree's parent is replaced or removed. The subtree is now kept aside and reused by a cache hit later in the same render, instead of making the next render fail, and a render that fails halfway no longer leaves unreachable elements in the tree.
