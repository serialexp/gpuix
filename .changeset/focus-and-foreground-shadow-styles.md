---
'@gpuix/native': minor
'@gpuix/react': minor
'@gpuix/solid': minor
---

New style keys:

- `focus` nested style, like `focusVisible`, but it applies for any focus, a mouse press included, like CSS `:focus`. Needs a focusable element
- `foregroundBoxShadow` takes the same one shadow or list as `boxShadow`, but paints over the element's descendants. It takes no layout space, so an inset one draws a focus ring over opaque children
- `visibility: "visible"` in a state style such as `hover` shows an element whose base style is hidden
