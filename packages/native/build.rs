extern crate napi_build;

use std::fs;
use std::path::{Path, PathBuf};

const PHOSPHOR_WEIGHTS: [&str; 6] = ["regular", "thin", "light", "bold", "fill", "duotone"];

fn main() {
    napi_build::setup();
    if std::env::var_os("CARGO_FEATURE_LUA54").is_some()
        || std::env::var_os("CARGO_FEATURE_LUAJIT").is_some()
    {
        pack_phosphor_icons();
    }
    let browser = std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32")
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("unknown");
    if std::env::var_os("CARGO_FEATURE_QUICKJS").is_some() && !browser {
        embed_modules();
    }
}

/// Checks the framework bundles `embedded/build.ts` wrote against the files
/// they were built from, and writes the table `src/quickjs/` embeds them
/// with. A stale bundle fails the build rather than being embedded.
fn embed_modules() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let manifest_path = manifest_dir.join("embedded/dist/manifest.txt");
    let rebuild = "run `bun run build:embedded` in packages/native";
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    let manifest = fs::read_to_string(&manifest_path)
        .unwrap_or_else(|error| panic!("reading {}: {error}; {rebuild}", manifest_path.display()));
    let mut recorded = None;
    let mut inputs = Vec::new();
    let mut modules = Vec::new();
    let mut files = Vec::new();
    for line in manifest.lines().filter(|line| !line.starts_with('#')) {
        let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
        match kind {
            "hash" => recorded = Some(rest),
            "input" => inputs.push(rest),
            "module" => modules.push(rest.split_once(' ').expect("module <specifier> <file>")),
            "file" => files.push(rest),
            _ => panic!(
                "{}: unexpected line {line:?}; {rebuild}",
                manifest_path.display()
            ),
        }
    }
    let recorded =
        recorded.unwrap_or_else(|| panic!("{} has no hash; {rebuild}", manifest_path.display()));

    // FNV-1a, 64-bit, over each input's path and bytes, each followed by a
    // zero byte, as `embedded/build.ts` computes it.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for input in &inputs {
        let path = manifest_dir.join(input);
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = fs::read(&path)
            .unwrap_or_else(|error| panic!("reading {}: {error}; {rebuild}", path.display()));
        for byte in input.bytes().chain([0]).chain(bytes).chain([0]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    let expected = format!("{hash:016x}");
    if recorded != expected {
        panic!("the embedded framework bundles are stale (built from {recorded}, sources are {expected}); {rebuild}");
    }

    let mut table = String::from("/// Import specifiers and the embedded file each one loads.\n");
    table.push_str("pub(crate) static MODULES: &[(&str, &str)] = &[\n");
    for (specifier, file) in &modules {
        table.push_str(&format!("    ({specifier:?}, {file:?}),\n"));
    }
    table.push_str("];\n\n/// Every embedded file, by its path under `embedded/dist/modules`.\n");
    table.push_str("pub(crate) static FILES: &[(&str, &str)] = &[\n");
    for file in &files {
        let path = manifest_dir.join("embedded/dist/modules").join(file);
        println!("cargo:rerun-if-changed={}", path.display());
        table.push_str(&format!(
            "    ({file:?}, include_str!({:?})),\n",
            path.display().to_string()
        ));
    }
    table.push_str("];\n");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("embedded_modules.rs");
    fs::write(&output, table)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", output.display()));
}

fn pack_phosphor_icons() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let package = manifest.join("../../node_modules/@phosphor-icons/core");
    let assets = package.join("assets");
    if !assets.is_dir() {
        panic!(
            "@phosphor-icons/core is missing at {}; run `bun install` from the repository root",
            package.display()
        );
    }

    println!(
        "cargo:rerun-if-changed={}",
        package.join("package.json").display()
    );
    println!("cargo:rerun-if-changed={}", assets.display());

    let aliases = phosphor_aliases(&package.join("dist/index.mjs"));
    let mut files_by_weight = Vec::with_capacity(PHOSPHOR_WEIGHTS.len());
    let mut icon_count = 0usize;
    for weight in PHOSPHOR_WEIGHTS {
        let mut files = fs::read_dir(assets.join(weight))
            .unwrap_or_else(|error| panic!("failed to read Phosphor {weight} icons: {error}"))
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "svg"))
            .collect::<Vec<_>>();
        files.sort();
        icon_count += files.len();
        files_by_weight.push(files);
    }

    let mut packed = Vec::with_capacity(5_000_000);
    packed.extend_from_slice(b"GPXPH1");
    write_u32(&mut packed, aliases.len());
    for (alias, canonical) in aliases {
        write_string(&mut packed, &alias);
        write_string(&mut packed, &canonical);
    }
    write_u32(&mut packed, icon_count);

    for (weight_index, files) in files_by_weight.into_iter().enumerate() {
        let weight = PHOSPHOR_WEIGHTS[weight_index];
        for path in files {
            let stem = path.file_stem().unwrap().to_string_lossy();
            let canonical = if weight == "regular" {
                stem.into_owned()
            } else {
                stem.strip_suffix(&format!("-{weight}"))
                    .unwrap_or(&stem)
                    .to_string()
            };
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
                .replace("currentColor", "#000");
            packed.push(weight_index as u8);
            write_string(&mut packed, &normalize_icon_name(&canonical));
            write_u32(&mut packed, source.len());
            packed.extend_from_slice(source.as_bytes());
        }
    }

    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("phosphor-icons.bin");
    fs::write(&output, packed)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", output.display()));
}

fn phosphor_aliases(catalog: &Path) -> Vec<(String, String)> {
    let source = fs::read_to_string(catalog)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", catalog.display()));
    let mut canonical = None;
    let mut aliases = Vec::new();
    for line in source.lines() {
        if let Some(name) = line
            .strip_prefix("    name: \"")
            .and_then(|line| line.split_once('\"'))
            .map(|(name, _)| name)
        {
            canonical = Some(normalize_icon_name(name));
            continue;
        }
        let Some(start) = line.find("alias: { name: \"") else {
            continue;
        };
        let alias = &line[start + "alias: { name: \"".len()..];
        let Some((alias, _)) = alias.split_once('\"') else {
            continue;
        };
        if let Some(canonical) = canonical.as_ref() {
            aliases.push((normalize_icon_name(alias), canonical.clone()));
        }
    }
    aliases
}

fn normalize_icon_name(name: &str) -> String {
    name.bytes()
        .filter(u8::is_ascii_alphanumeric)
        .map(|byte| byte.to_ascii_lowercase() as char)
        .collect()
}

fn write_string(output: &mut Vec<u8>, value: &str) {
    let len = u16::try_from(value.len()).expect("Phosphor icon name is too long");
    output.extend_from_slice(&len.to_le_bytes());
    output.extend_from_slice(value.as_bytes());
}

fn write_u32(output: &mut Vec<u8>, value: usize) {
    let value = u32::try_from(value).expect("Phosphor icon pack is too large");
    output.extend_from_slice(&value.to_le_bytes());
}
