# Current task

Enable `TestGpuixRenderer` and screenshot capture on Linux without creating native desktop windows.

## State

Implemented. Linux now uses GPUI `HeadlessAppContext` + a surface-free WGPU `PlatformHeadlessRenderer`. It creates no X11/Wayland window, renders into a reusable offscreen texture, and reads screenshots through a reusable staging buffer.

## Verification

- Focused Lua focus test: 2 passed.
- Focused React screenshot/color test: 37 passed.
- Window-tree probe around both focused runs: 532 entries before and after; no test window created.
- Full React suite: 25 files passed, 376 tests passed, 1 intentionally skipped.
- Native Rust tests: 275 passed.
- Native Vitest: 2 passed.
- GPUI WGPU tests: 30 passed.
- React TypeScript build passed.
- GPUI platform check with `test-support,wayland,x11` passed.
- Diff checks passed.

## Repository safety

The recovery stash remains at `stash@{0}`. `packages/native/index.d.ts` and `packages/native/tests/lua-focus.test.ts` still have pre-existing staged state from stash application; the build regenerated `index.d.ts`, so it is currently modified in both index and working tree. No commit was made.

The Zed submodule contains the fork changes as uncommitted modifications. Its required `README.md` review notice was already present and remains present.
