//! Which framework an app file's JSX is compiled for. As TypeScript reads
//! it, the nearest `tsconfig.json` (or `jsconfig.json`) above the file
//! decides, by its `compilerOptions.jsxImportSource`, `extends` included:
//! `@gpuix/solid` is Solid and `@gpuix/react` is React. Solid apps set it
//! anyway, so their editor finds `@gpuix/solid`'s JSX types. A file's own
//! `@jsxImportSource` comment isn't read.
//!
//! A file with no such setting goes with the app's entry file: the app is
//! Solid if the entry imports `@gpuix/solid` or `solid-js`, and React
//! otherwise.

use std::cell::OnceCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use oxc::allocator::Allocator;
use oxc::ast::ast::Statement;
use oxc::parser::Parser;
use serde_json::Value;

use super::compile::{source_type, Jsx};

const CONFIG_FILES: [&str; 2] = ["tsconfig.json", "jsconfig.json"];

/// What one runtime has worked out so far. A reload starts a new runtime, so
/// a changed config counts from the next reload.
#[derive(Default)]
pub(crate) struct Frameworks {
    /// The framework each directory's nearest config sets, if it sets one.
    directories: HashMap<PathBuf, Option<Jsx>>,
    /// The entry file's framework, once a file needed it.
    entry: OnceCell<Jsx>,
}

impl Frameworks {
    /// The framework for the app file at `path`. `entry` is the app's entry
    /// module and reads its source when no config decides.
    pub(crate) fn for_file(
        &mut self,
        path: &Path,
        entry: impl FnOnce() -> Option<(PathBuf, String)>,
    ) -> Result<Jsx, String> {
        let directory = path.parent().unwrap_or(Path::new(""));
        if let Some(jsx) = self.configured(directory)? {
            return Ok(jsx);
        }
        Ok(*self.entry.get_or_init(|| match entry() {
            Some((path, source)) => entry_framework(&path, &source),
            None => Jsx::React,
        }))
    }

    /// The framework the nearest config at or above `directory` sets.
    fn configured(&mut self, directory: &Path) -> Result<Option<Jsx>, String> {
        if let Some(jsx) = self.directories.get(directory) {
            return Ok(*jsx);
        }
        let config = CONFIG_FILES
            .iter()
            .map(|name| directory.join(name))
            .find(|path| path.is_file());
        let jsx = match (config, directory.parent()) {
            (Some(config), _) => read_config(&config)?,
            (None, Some(parent)) => self.configured(parent)?,
            (None, None) => None,
        };
        self.directories.insert(directory.to_path_buf(), jsx);
        Ok(jsx)
    }
}

/// The framework `config`'s `jsxImportSource` names, set in it or in what
/// it extends.
fn read_config(config: &Path) -> Result<Option<Jsx>, String> {
    let Some(source) = import_source(config, &mut Vec::new())? else {
        return Ok(None);
    };
    [Jsx::React, Jsx::Solid]
        .into_iter()
        .find(|jsx| jsx.import_source() == source)
        .map(Some)
        .ok_or_else(|| {
            format!(
                "{}: gpx compiles JSX for {} or {}, but jsxImportSource is {source:?}",
                config.display(),
                Jsx::React.import_source(),
                Jsx::Solid.import_source(),
            )
        })
}

/// `compilerOptions.jsxImportSource` in `config`, or else in the configs it
/// extends, the last of them first, as TypeScript merges them. `chain` is
/// the configs that extend this one, to find a loop; two configs may both
/// extend a third.
fn import_source(config: &Path, chain: &mut Vec<PathBuf>) -> Result<Option<String>, String> {
    let config = fs::canonicalize(config).unwrap_or_else(|_| config.to_path_buf());
    if chain.contains(&config) {
        return Err(format!("{}: `extends` loops back to it", config.display()));
    }
    chain.push(config.clone());
    let source = own_or_extended_import_source(&config, chain);
    chain.pop();
    source
}

fn own_or_extended_import_source(
    config: &Path,
    chain: &mut Vec<PathBuf>,
) -> Result<Option<String>, String> {
    let text = fs::read_to_string(config)
        .map_err(|error| format!("Cannot read {}: {error}", config.display()))?;
    let json: Value = serde_json::from_str(&jsonc_to_json(&text))
        .map_err(|error| format!("{}: {error}", config.display()))?;
    if let Some(source) = json.pointer("/compilerOptions/jsxImportSource") {
        return match source {
            Value::String(source) => Ok(Some(source.clone())),
            _ => Err(format!(
                "{}: jsxImportSource must be a string",
                config.display()
            )),
        };
    }
    let extends: Vec<&str> = match json.get("extends") {
        None => Vec::new(),
        Some(Value::String(name)) => vec![name],
        Some(Value::Array(names)) => names.iter().filter_map(Value::as_str).collect(),
        Some(_) => {
            return Err(format!(
                "{}: `extends` must be a string or a list of strings",
                config.display()
            ))
        }
    };
    let directory = config.parent().unwrap_or(Path::new(""));
    for name in extends.into_iter().rev() {
        let base = find_extended(directory, name).ok_or_else(|| {
            format!(
                "{}: cannot find {name:?}, which it extends",
                config.display()
            )
        })?;
        if let Some(source) = import_source(&base, chain)? {
            return Ok(Some(source));
        }
    }
    Ok(None)
}

/// The config `extends: name` names, from a config in `directory`: a path,
/// with `.json` implied, or a package in a `node_modules` directory above,
/// either a file in it or its `tsconfig.json`.
fn find_extended(directory: &Path, name: &str) -> Option<PathBuf> {
    let with_json = |path: PathBuf| {
        let mut json = path.clone().into_os_string();
        json.push(".json");
        [path, PathBuf::from(json)]
            .into_iter()
            .find(|path| path.is_file())
    };
    if name.starts_with("./") || name.starts_with("../") || Path::new(name).is_absolute() {
        return with_json(directory.join(name));
    }
    directory.ancestors().find_map(|ancestor| {
        let package = ancestor.join("node_modules").join(name);
        with_json(package.clone()).or_else(|| {
            let config = package.join("tsconfig.json");
            config.is_file().then_some(config)
        })
    })
}

/// The framework an entry file's imports name: Solid if it imports
/// `@gpuix/solid` or `solid-js` (or a subpath of either), React otherwise.
/// A file that doesn't parse is React; compiling it reports why.
fn entry_framework(path: &Path, source: &str) -> Jsx {
    let Ok(source_type) = source_type(path) else {
        return Jsx::React;
    };
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    let solid = parsed.program.body.iter().any(|statement| {
        let specifier = match statement {
            Statement::ImportDeclaration(it) => &it.source.value,
            Statement::ExportAllDeclaration(it) => &it.source.value,
            Statement::ExportFromDeclaration(it) => &it.source.value,
            _ => return false,
        };
        ["@gpuix/solid", "solid-js"].iter().any(|package| {
            specifier
                .strip_prefix(package)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        })
    });
    if solid {
        Jsx::Solid
    } else {
        Jsx::React
    }
}

/// JSON with comments and trailing commas, as TypeScript reads its configs,
/// as plain JSON. Each removed character becomes a space (line breaks stay),
/// so a JSON error's line and column are still the file's.
fn jsonc_to_json(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = bytes.to_vec();
    let mut index = 0;
    // Where the last comma outside a string is, until something other than
    // space or a comment follows it.
    let mut comma: Option<usize> = None;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                comma = None;
                index += 1;
                while index < bytes.len() && bytes[index] != b'"' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
                index += 1;
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    out[index] = b' ';
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let end = text[index + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |end| index + 2 + end + 2);
                for byte in &mut out[index..end] {
                    if *byte != b'\n' {
                        *byte = b' ';
                    }
                }
                index = end;
            }
            b',' => {
                comma = Some(index);
                index += 1;
            }
            b'}' | b']' => {
                if let Some(comma) = comma.take() {
                    out[comma] = b' ';
                }
                index += 1;
            }
            byte => {
                if !byte.is_ascii_whitespace() {
                    comma = None;
                }
                index += 1;
            }
        }
    }
    // Only ASCII bytes outside strings were replaced, by ASCII spaces.
    String::from_utf8(out).expect("replacing ASCII bytes keeps UTF-8 valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonc_loses_comments_and_trailing_commas_but_keeps_positions() {
        let text = "{\n  // a comment, with a comma\n  \"a\": \"//not a comment\", /* x\n */ \"b\": [1, 2,],\n}";
        let json = jsonc_to_json(text);
        assert_eq!(json.len(), text.len());
        assert_eq!(json.lines().count(), text.lines().count());
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["a"], "//not a comment");
        assert_eq!(value["b"], serde_json::json!([1, 2]));
        assert_eq!(jsonc_to_json(r#"{"a": "\",}"}"#), r#"{"a": "\",}"}"#);
    }

    #[test]
    fn the_entry_imports_name_the_framework() {
        let entry = |source: &str| entry_framework(Path::new("main.tsx"), source);
        assert_eq!(
            entry(r#"import { render } from "@gpuix/solid""#),
            Jsx::Solid
        );
        assert_eq!(
            entry(r#"import { createSignal } from "solid-js""#),
            Jsx::Solid
        );
        assert_eq!(entry(r#"export * from "@gpuix/solid/select""#), Jsx::Solid);
        assert_eq!(
            entry(r#"import { render } from "@gpuix/react""#),
            Jsx::React
        );
        assert_eq!(entry(r#"import "@gpuix/solidity""#), Jsx::React);
        assert_eq!(entry("const a = 1"), Jsx::React);
    }
}
