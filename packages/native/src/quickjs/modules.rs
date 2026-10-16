//! How `import` finds modules. A relative or absolute specifier names an app
//! file on disk, compiled on load by [`compile`], with its JSX for the
//! framework `framework.rs` picks. A bare specifier names one
//! of the framework bundles `embedded/build.ts` writes and `build.rs` embeds
//! (`react`, `@gpuix/react` and the rest). Apps can import nothing else yet.
//!
//! App modules are named by their canonical path, so a file reached by two
//! relative paths is one module. Embedded ones are named `gpuix:` and their
//! path in the bundle, and their relative imports stay inside it.
//!
//! As in Bun, an app file imported `with { type: "text" }` is a module whose
//! default export is the file's text. It is named `text:` and the canonical
//! path, so the same file can also be imported as code. App modules get
//! `import.meta.url` and `import.meta.main`, which is true for the entry.
//!
//! QuickJS calls both from C, where a panic can't unwind, so each runs in
//! [`guard`].

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::Declared;
use rquickjs::{Ctx, Exception, Module};

use super::compile::compile;
use super::{guard, throw_whole, ErrorKind, Shared};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_modules.rs"));
}

const EMBEDDED: &str = "gpuix:";
const TEXT: &str = "text:";

/// The extensions an import may leave out, in the order Bun tries them for
/// an `import` statement.
const IMPLIED_EXTENSIONS: [&str; 6] = ["tsx", "jsx", "mts", "ts", "mjs", "js"];

pub(crate) struct AppResolver(pub Rc<Shared>);

impl Resolver for AppResolver {
    fn resolve<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        guard(&self.0, ctx, "import", || {
            let kind = match attributes {
                Some(attributes) => attributes.get_type()?,
                None => None,
            };
            let resolved = match kind.as_deref() {
                None => resolve(base, name),
                Some("text") => resolve_text(base, name),
                Some(kind) => Err(format!(
                    "Cannot import {name:?} from {base}: `type: {kind:?}` imports aren't supported, only `type: \"text\"`"
                )),
            };
            resolved.map_err(|message| Exception::throw_message(ctx, &message))
        })
    }
}

pub(crate) struct AppLoader(pub Rc<Shared>);

impl Loader for AppLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        guard(&self.0, ctx, "import", || {
            if let Some(file) = name.strip_prefix(EMBEDDED) {
                let source = embedded_file(file).ok_or_else(|| {
                    Exception::throw_message(ctx, &format!("{name} isn't in the embedded bundle"))
                })?;
                return Module::declare(ctx.clone(), name, source);
            }
            if let Some(file) = name.strip_prefix(TEXT) {
                // As `TextDecoder` and `Response.text()` do, bytes that aren't
                // UTF-8 become U+FFFD rather than failing the import.
                let bytes = fs::read(file).map_err(|error| cannot_read(ctx, file, error))?;
                let text = String::from_utf8_lossy(&bytes);
                let literal = serde_json::to_string(&text).map_err(|error| {
                    Exception::throw_message(ctx, &format!("Cannot import {file}: {error}"))
                })?;
                return Module::declare(ctx.clone(), name, format!("export default {literal};"));
            }
            let path = Path::new(name);
            let source = read(ctx, name)?;
            let jsx = || {
                self.0.frameworks.borrow_mut().for_file(path, || {
                    let entry = self.0.entry.as_deref()?;
                    let source = if entry == name {
                        source.clone()
                    } else {
                        fs::read_to_string(entry).ok()?
                    };
                    Some((PathBuf::from(entry), source))
                })
            };
            let compiled = compile(&source, path, jsx)
                .map_err(|message| throw_whole(ctx, ErrorKind::Syntax, &message))?;
            self.0
                .positions
                .borrow_mut()
                .insert(name.to_string(), compiled.positions);
            let module = Module::declare(ctx.clone(), name, compiled.code)?;
            let meta = module.meta()?;
            if let Ok(url) = url::Url::from_file_path(path) {
                meta.set("url", url.as_str())?;
            }
            meta.set("main", self.0.entry.as_deref() == Some(name))?;
            Ok(module)
        })
    }
}

fn read<'js>(ctx: &Ctx<'js>, file: &str) -> rquickjs::Result<String> {
    fs::read_to_string(file).map_err(|error| cannot_read(ctx, file, error))
}

fn cannot_read(ctx: &Ctx<'_>, file: &str, error: std::io::Error) -> rquickjs::Error {
    Exception::throw_message(ctx, &format!("Cannot read {file}: {error}"))
}

fn embedded_file(file: &str) -> Option<&'static str> {
    embedded::FILES
        .iter()
        .find(|(path, _)| *path == file)
        .map(|(_, source)| *source)
}

/// The module name `name` gets when `base` imports it. An empty `base` is
/// the entry, which `QuickJsRuntime::load_module` also names with this.
pub(super) fn resolve(base: &str, name: &str) -> Result<String, String> {
    let relative = name.starts_with("./") || name.starts_with("../") || name == "." || name == "..";
    if let Some(file) = base.strip_prefix(EMBEDDED) {
        if relative {
            return embedded_join(file, name)
                .filter(|file| embedded_file(file).is_some())
                .map(|file| format!("{EMBEDDED}{file}"))
                .ok_or_else(|| {
                    format!("Cannot import {name:?} from {base}: not in the embedded bundle")
                });
        }
    } else if relative || Path::new(name).is_absolute() {
        let directory = Path::new(base).parent().unwrap_or(Path::new(""));
        let target = directory.join(name);
        let found = find_app_file(&target).ok_or_else(|| {
            if base.is_empty() {
                format!("Cannot find {}", target.display())
            } else {
                format!(
                    "Cannot find {name:?} imported from {base} (looked for {})",
                    target.display()
                )
            }
        })?;
        let canonical = fs::canonicalize(&found)
            .map_err(|error| format!("Cannot import {}: {error}", found.display()))?;
        return canonical.into_os_string().into_string().map_err(|path| {
            format!(
                "Cannot import {}: the path isn't UTF-8",
                path.to_string_lossy()
            )
        });
    }
    embedded::MODULES
        .iter()
        .find(|(specifier, _)| *specifier == name)
        .map(|(_, file)| format!("{EMBEDDED}{file}"))
        .ok_or_else(|| {
            let bundled: Vec<&str> = embedded::MODULES.iter().map(|(specifier, _)| *specifier).collect();
            format!(
                "Cannot import {name:?} from {base}: apps can import their own files and the bundled modules ({})",
                bundled.join(", ")
            )
        })
}

/// The module name of an app file imported `with { type: "text" }`. As in
/// Bun, the import names the file itself: no extension is implied.
fn resolve_text(base: &str, name: &str) -> Result<String, String> {
    let relative = name.starts_with("./") || name.starts_with("../");
    if base.starts_with(EMBEDDED) || !(relative || Path::new(name).is_absolute()) {
        return Err(format!(
            "Cannot import {name:?} from {base} as text: only the app's own files can be"
        ));
    }
    let target = Path::new(base).parent().unwrap_or(Path::new("")).join(name);
    let canonical = fs::canonicalize(&target).map_err(|error| {
        format!(
            "Cannot find {name:?} imported from {base} (looked for {}): {error}",
            target.display()
        )
    })?;
    if !canonical.is_file() {
        return Err(format!(
            "Cannot import {name:?} from {base} as text: {} isn't a file",
            canonical.display()
        ));
    }
    canonical
        .into_os_string()
        .into_string()
        .map(|path| format!("{TEXT}{path}"))
        .map_err(|path| {
            format!(
                "Cannot import {}: the path isn't UTF-8",
                path.to_string_lossy()
            )
        })
}

/// The file an app import names, as Bun and TypeScript find it: the path
/// itself, a `.js`-style path naming its TypeScript source, the path with a
/// module extension added, or a directory's `index` file.
fn find_app_file(target: &Path) -> Option<PathBuf> {
    if target.is_file() {
        return Some(target.to_path_buf());
    }
    let sources: &[&str] = match target.extension().and_then(|extension| extension.to_str()) {
        Some("js") => &["ts", "tsx"],
        Some("jsx") => &["tsx"],
        Some("mjs") => &["mts"],
        _ => &[],
    };
    let swapped = sources
        .iter()
        .map(|extension| target.with_extension(extension));
    let appended = IMPLIED_EXTENSIONS.iter().map(|extension| {
        let mut path = target.as_os_str().to_owned();
        path.push(".");
        path.push(extension);
        PathBuf::from(path)
    });
    let index = IMPLIED_EXTENSIONS
        .iter()
        .map(|extension| target.join(format!("index.{extension}")));
    swapped
        .chain(appended)
        .chain(index)
        .find(|path| path.is_file())
}

/// `name` relative to the embedded file `base`, or `None` if it leaves the
/// bundle.
fn embedded_join(base: &str, name: &str) -> Option<String> {
    let mut parts: Vec<&str> = base.split('/').collect();
    parts.pop();
    for component in Path::new(name).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::Normal(part) => parts.push(part.to_str()?),
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::embedded_join;

    #[test]
    fn relative_imports_stay_inside_the_bundle() {
        assert_eq!(
            embedded_join("@gpuix/react/select.js", "../chunk-1.js").as_deref(),
            Some("@gpuix/chunk-1.js")
        );
        assert_eq!(
            embedded_join("react.js", "./chunk-1.js").as_deref(),
            Some("chunk-1.js")
        );
        assert_eq!(embedded_join("react.js", "../outside.js"), None);
        assert_eq!(embedded_join("@gpuix/react.js", "../../etc/passwd"), None);
        assert_eq!(embedded_join("react.js", "/etc/passwd"), None);
    }
}
