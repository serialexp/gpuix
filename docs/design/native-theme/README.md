# Native theme design (pulled from Claude Design)

Local copy of the Claude Design project
https://claude.ai/design/p/28bd9d65-16e9-440d-9f69-a9c402d0d22f, pulled on
2026-10-22 (re-pulled the same day for the Network tab) so agents without Claude Design access can read it.

- `Native Theme Spec.dc.html` is the chosen spec: direction "Comet, tidied",
  the full dark/light token table for `Theme::dark()` / `Theme::light()`
  (tokens marked `new` don't exist in `theme.rs` yet), component previews for
  the Lua widgets, and the metrics list. The inspector mock at the top has
  Elements and Network tabs (the page opens on Network; click the tabs and
  request rows to switch). The Network tab data is in `REQS` and `mode()`. The token values and metrics live in
  the `<script data-dc-script>` block at the bottom (`D`, `L`, `GROUPS`,
  `METRICS`), which is the easiest place to read them as data.
- `Test Runner.dc.html` is the test results window: a summary bar, then
  three columns (tests, steps of the selected test, and step details with
  args, an expected/received error box and a snapshot of the app frame with
  the target element ringed). It has `theme` (Dark/Light) and `scenario` (One
  failure/All passing) props in its `data-props`, and arrow keys move between
  steps and tests. Everything it renders is computed in `renderVals()`.
- `Theme Board.dc.html` shows the three directions that were compared (1a was
  picked).
- `github.md` is the project's sync note and screen-to-repo-file map.
- `support.js` is the generated Claude Design runtime the pages need to render.

To view the pages, serve this directory over HTTP (the runtime re-fetches the
page, so `file://` won't work) and have network access for React, Babel and
Phosphor icons from unpkg:

    python3 -m http.server -d docs/design/native-theme 8765
    # open http://localhost:8765/Native%20Theme%20Spec.dc.html
