# Vendored crates

## rquickjs-sys 0.14.0

The crates.io package of `rquickjs-sys` 0.14.0 with
`rquickjs-sys-0.14.0-map-hash-mix.diff` applied. `Cargo.toml` swaps it in
with `[patch.crates-io]`, and the `quickjs` feature turns the fix on.

The patch adds a `map-hash-mix` feature that defines `RQUICKJS_MAP_HASH_MIX`,
and with it a final mix (the MurmurHash3 finalizer) in quickjs-ng's
`map_hash_key`. quickjs-ng takes a Map or Set bucket from the low bits of the
key's hash, and those bits are constant for small integers and for object
pointers, so most such keys share a few buckets and lookups degrade towards a
linear scan. Solid's `For` keys a Map by both. The patch also silences an
unused-variable C warning that only shows with assertions disabled.

The package's `vendor/wasi-libc` directory is left out, so WASI targets
don't build. Nothing else differs.

To rebuild it, for example for a new rquickjs-sys version:

```sh
rm -rf vendor/rquickjs-sys
cp -R ~/.cargo/registry/src/index.crates.io-*/rquickjs-sys-0.14.0 vendor/rquickjs-sys
rm -rf vendor/rquickjs-sys/vendor
(cd vendor/rquickjs-sys && patch -p1 < ../rquickjs-sys-0.14.0-map-hash-mix.diff)
```
