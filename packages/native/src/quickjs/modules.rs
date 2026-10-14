//! How `import` finds modules. A relative or absolute specifier names an app
//! file on disk, compiled on load by [`compile`]. A bare specifier names one
//! of the framework bundles `embedded/build.ts` writes and `build.rs` embeds
//! (`react`, `@gpuix/react` and the rest). Apps can import nothing else yet.
//!
//! App modules are named by their canonical path, so a file reached by two
//! relative paths is one module. Embedded ones are named `gpuix:` and their
//! path in the bundle, and their relative imports stay inside it.
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
            if let Some(kind) = kind {
                return Err(Exception::throw_message(
                    ctx,
                    &format!(
                        "Cannot import {name:?} from {base}: `type: {kind:?}` imports aren't supported"
                    ),
                ));
            }
            resolve(base, name).map_err(|message| Exception::throw_message(ctx, &message))
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
            let path = Path::new(name);
            let source = fs::read_to_string(path).map_err(|error| {
                Exception::throw_message(ctx, &format!("Cannot read {name}: {error}"))
            })?;
            let compiled = compile(&source, path)
                .map_err(|message| throw_whole(ctx, ErrorKind::Syntax, &message))?;
            self.0
                .positions
                .borrow_mut()
                .insert(name.to_string(), compiled.positions);
            Module::declare(ctx.clone(), name, compiled.code)
        })
    }
}

fn embedded_file(file: &str) -> Option<&'static str> {
    embedded::FILES
        .iter()
        .find(|(path, _)| *path == file)
        .map(|(_, source)| *source)
}

fn resolve(base: &str, name: &str) -> Result<String, String> {
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
