---
'@gpuix/native': minor
---

`gpx --watch` gives React apps Fast Refresh. It runs the frameworks' development builds, compiles React files with React Refresh's registrations, and, when a file changes, runs it again and lets React swap its components in place, keeping their state. A file that exports more than components runs again in the files that import it, up to ones that export only components; a change that reaches the entry file, or a Solid app's change, starts the app again in the same window. Each file runs once per update, after the new versions of what it imports. A file that fails to compile or throws during an update is reported, and the app runs on as it was; the next update brings in the rest of that one's changes. Without `--watch`, `gpx` runs the production builds as before.
