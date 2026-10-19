# Vendored crates

## rquickjs-sys 0.14.0

The crates.io package of `rquickjs-sys` 0.14.0 with
`rquickjs-sys-0.14.0-map-hash-mix.diff`,
`rquickjs-sys-0.14.0-source-position.diff`,
`rquickjs-sys-0.14.0-proxy-accessor-descriptor.diff` and
`rquickjs-sys-0.14.0-rerun-on-source-change.diff` applied, in that order.
`Cargo.toml` swaps it in with `[patch.crates-io]`, and the `quickjs` feature
turns on the first two's features. The other two are plain fixes.

### map-hash-mix

The patch adds a `map-hash-mix` feature that defines `RQUICKJS_MAP_HASH_MIX`,
and with it a final mix (the MurmurHash3 finalizer) in quickjs-ng's
`map_hash_key`. quickjs-ng takes a Map or Set bucket from the low bits of the
key's hash, and those bits are constant for small integers and for object
pointers, so most such keys share a few buckets and lookups degrade towards a
linear scan. Solid's `For` keys a Map by both. `map-hash-repro.js` shows
it: run it with quickjs-ng's `qjs` (or `gpx --check`, which has the fix),
and integer and object keys only stay as fast as strings with the patch. The patch also silences an
unused-variable C warning that only shows with assertions disabled.

### source-position

The patch adds a `source-position` feature that defines
`RQUICKJS_SOURCE_POSITION`, and with it `JS_SetSourcePositionFunc`, a hook
that maps each position a stack trace shows. GPUIX compiles app TypeScript
and JSX before QuickJS runs it, so QuickJS's positions are in the compiled
code; the hook puts them back at the source. It applies to the built-in
stack format, to the `CallSite`s an app's `Error.prepareStackTrace` gets,
and to a syntax error's position where QuickJS shows one. A hook written in
JavaScript (as `Error.prepareStackTrace`) can't run when the stack is full,
and QuickJS then leaves a stack overflow's `stack` null; a C hook can. That
still happens when an app sets its own `Error.prepareStackTrace`, as it
does in stock QuickJS. The nonstandard `Function.prototype.lineNumber` and
`columnNumber` aren't mapped.

### proxy-accessor-descriptor

quickjs-ng drops the getter from a descriptor a Proxy's
`getOwnPropertyDescriptor` trap returns: `js_obj_to_desc` notes that `get`
or `set` is present, but `js_proxy_get_own_property` passes that on without
marking the descriptor an accessor, so `Object.getOwnPropertyDescriptor`
reads back a data descriptor with no value. Solid 2.0's `merge` and `omit`
answer with getters, so props that go through both lost their children.
The patch converts the flags as Bellard's QuickJS does. quickjs-ng upstream
still has the bug.

### rerun-on-source-change

The build script tells Cargo when to rerun it, but only for environment
variables, and that stops Cargo from rerunning it when a file changes. So a
change to the vendored C, as these patches make, wasn't built until a clean
build. The patch names the C sources too.

The package's `vendor/wasi-libc` directory is left out, so WASI targets
don't build. Nothing else differs.

To rebuild it, for example for a new rquickjs-sys version:

```sh
rm -rf vendor/rquickjs-sys
cp -R ~/.cargo/registry/src/index.crates.io-*/rquickjs-sys-0.14.0 vendor/rquickjs-sys
rm -rf vendor/rquickjs-sys/vendor
(cd vendor/rquickjs-sys && patch -p1 < ../rquickjs-sys-0.14.0-map-hash-mix.diff)
(cd vendor/rquickjs-sys && patch -p1 < ../rquickjs-sys-0.14.0-source-position.diff)
(cd vendor/rquickjs-sys && patch -p1 < ../rquickjs-sys-0.14.0-proxy-accessor-descriptor.diff)
(cd vendor/rquickjs-sys && patch -p1 < ../rquickjs-sys-0.14.0-rerun-on-source-change.diff)
```
