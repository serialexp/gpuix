# Native application packaging

Implemented 2026-10-21. This supersedes the compile-time payload and Bun/Cargo
packaging prototype. `gpx package` needs the installed runner and the user's app,
not a GPUIX source checkout or compiler.

## Runner plus archive

The packer copies its own executable into private staging storage. The copy
contains the same runtimes, framework libraries, native host and packaging code.
It collects explicitly included application files into a versioned JSON archive
with base64 file contents, then inserts the archive into the copied executable.
This is not compilation, obfuscation, permission enforcement or cross-compilation.

The archive ends with a 32-byte footer:

| Bytes | Meaning |
| --- | --- |
| 0–7 | Archive byte length, little-endian u64 |
| 8–15 | Original runner length before archive/alignment, little-endian u64 |
| 16–31 | ASCII `GPUIX-PAYLOAD-01` |

Padding before the archive aligns the footer's end to 16 bytes. The original
length lets repackaging remove the previous archive rather than nesting copies.
Startup reads the executable header, footer and archive, not the entire runner.
The encoded archive is capped at 512 MiB; input files at 384 MiB. JSON overhead
can make an input near that limit too large, in which case packaging fails.

Archive paths must be relative and cannot traverse parents or contain drive
prefixes. Collection rejects symlinks, including symlinked parent directories,
and non-regular files. Extraction rejects duplicate paths and missing entries.
It creates only ordinary files inside an owned private temporary directory. That
directory remains alive through the application loop; normal exit removes it.
These bounds protect the container format, not a sandbox for application code.

## Platform executable formats

- **ELF:** the archive is an overlay after the original executable. Existing
  loadable segments are unchanged.
- **PE32+:** an existing trailing Authenticode certificate is removed and its
  security-directory entry cleared before inserting the archive. Outputs are
  unsigned. Non-trailing certificate layouts are rejected; subsequent Windows
  signing is not yet validated.
- **Thin 64-bit Mach-O:** remove the old signature with `codesign`, then extend
  `__LINKEDIT`'s file size and page-aligned virtual size to cover the archive.
  Ad-hoc signing happens afterward. The reader uses `LC_CODE_SIGNATURE`'s file
  offset as the archive boundary, allowing up to 15 alignment bytes before it.
  Later re-signing can replace the signature without losing the archive.
  Universal/fat Mach-O is explicitly unsupported.

No native image modifications are made to the installed source runner. The
staged copy is writable/executable regardless of the installed runner's mode.

## Bundle formats and publication

Native `.app` construction writes the plist, icon and executable directly, then
signs and verifies the bundle with Apple's tools. Native `.deb` construction
writes the package tree, validates its desktop entry, derives dependencies with
`dpkg-shlibdeps`, adds GPUI's runtime-loaded libraries, and invokes `dpkg-deb`.
Default app icons are embedded in the runner. Format tools remain prerequisites;
there is no hidden source-build fallback when a format is unavailable.

Before publication, the staged application runs `--check` in an isolated working
directory with inherited Lua/Node module paths removed and a 60-second limit.
This checks application startup, not painting or every lazy import. If validation
fails, the previous output is unchanged. Successful publication uses staged
renames in the output's filesystem, with rollback if replacement fails. A crash
or forced termination can leave staging/backup directories behind.

## Validation

Native unit tests cover archive replacement, malformed bounds, Mach-O signature
boundaries, PE certificate removal, collection and metadata. The desktop workflow
runs `scripts/check-native-packager.ts`: an installed runner packages Lua and
React/TypeScript apps without Bun/Cargo on PATH, exercises embedded imports and
binary assets, repackages, preserves an old output on failure, and checks macOS
bundles with default embedded icons. Source files are removed before the final
startup check. Linux CI additionally installs and removes the real `.deb`.

Local macOS checks include re-signing with hardened-runtime options. A Debian
arm64 container has exercised these same Rust packaging modules through a small
non-GPUI runner harness, including an apt install/check/remove cycle. Full Linux
and Windows application behavior still requires the native CI jobs; GUI painting
and input need separate tests.
