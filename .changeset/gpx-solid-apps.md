---
'@gpuix/native': minor
---

`gpx` runs Solid apps. `solid-js` and `@gpuix/solid` with its components are bundled in, using Solid's production build (`@gpuix/solid/testing` and the Bun-only subpaths aren't), and app files with JSX compile with Solid's own compiler. The nearest `tsconfig.json` or `jsconfig.json` picks the framework by its `jsxImportSource` (`@gpuix/solid` or `@gpuix/react`; anything else is an error). A file without JSX ignores the setting. Without a config, an app whose entry file imports `@gpuix/solid` or `solid-js` is Solid, and any other app is React. Errors and stack traces point at the source lines.

Fixes along the way: an empty text node, such as the placeholder Solid puts where a `<Show>` or `{null}` renders nothing, no longer takes a `gap` in its parent's layout. And under `gpx`, a Proxy whose `getOwnPropertyDescriptor` trap returns a getter now reports that getter, rather than a data property with no value.
