# Tray demos

Run `just start-tray` for Lua or `just start-tray-js` for React.
These read `gpuix.json` and `gpuix-js.json`, respectively.
Package either with `just package examples/tray/gpuix.json` or
`just package examples/tray/gpuix-js.json`. The resulting executable embeds
the selected entry and `icon.svg`; no source folder is required at launch.

The two icon assets are independent:

- `icon.svg` is the menu-bar icon, selected by the application's tray options.
- `../../scripts/assets/AppIcon.icns` is the Dock/Finder/app-switcher icon,
  selected by the manifest's `icon` setting (or the default) before launch.

They currently share artwork, but can be completely different. Changing the
tray icon does not change the application icon, and vice versa.
