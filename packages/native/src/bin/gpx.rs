//! Runs a GPUIX app: a Lua or LuaX app in the embedded Lua runtime, or a
//! JavaScript, TypeScript or JSX app in the embedded QuickJS runtime. The
//! entry file's extension picks the engine.

use std::path::{Path, PathBuf};

#[path = "gpx/archive.rs"]
mod archive;
#[path = "gpx/package.rs"]
mod package;
#[path = "gpx/packager.rs"]
mod packager;

use gpuix_native::{
    check_js_file, check_lua_file, is_js_source, is_lua_source, run_js_file, run_lua_file,
    AppOptions, JS_EXTENSIONS, LUA_EXTENSIONS,
};

/// The entry files a packaged app may hold, in the order they are looked for.
const BUNDLED_ENTRIES: [&str; 6] = [
    "main.luax",
    "main.lua",
    "main.tsx",
    "main.ts",
    "main.jsx",
    "main.js",
];

#[derive(Debug, PartialEq)]
enum Engine {
    Lua,
    QuickJs,
}

struct Arguments {
    path: PathBuf,
    engine: Engine,
    options: AppOptions,
    check: bool,
}

pub(crate) fn main() {
    if let Err(error) = run() {
        eprintln!("gpx: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let raw_arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if raw_arguments.first().map(String::as_str) == Some("package") {
        return packager::run(&raw_arguments[1..]);
    }
    if raw_arguments
        .iter()
        .any(|argument| argument == "-h" || argument == "--help")
    {
        println!("{}", usage());
        return Ok(());
    }
    let mut raw_arguments = raw_arguments;
    let manifest_position = raw_arguments
        .iter()
        .position(|argument| argument == "--manifest");
    let manifest_path = if let Some(position) = manifest_position {
        if position + 1 >= raw_arguments.len() {
            return Err("--manifest requires a path".into());
        }
        let path = PathBuf::from(raw_arguments.remove(position + 1));
        raw_arguments.remove(position);
        Some(std::fs::canonicalize(path).map_err(|error| error.to_string())?)
    } else {
        None
    };
    let embedded = if manifest_path.is_none() {
        package::EmbeddedApp::load()?
    } else {
        None
    };
    let manifest = manifest_path
        .as_ref()
        .map(|path| package::Manifest::read(path))
        .transpose()?;
    let active_manifest = manifest
        .as_ref()
        .or_else(|| embedded.as_ref().map(|app| &app.manifest));
    let root = manifest_path
        .as_ref()
        .and_then(|path| path.parent())
        .or_else(|| embedded.as_ref().map(|app| app.root()));
    let configured_source = active_manifest
        .zip(root)
        .map(|(manifest, root)| root.join(&manifest.entry));
    let bundled_source = configured_source.or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|path| bundled_source_path(&path))
    });
    let mut arguments = parse_arguments_with_default(raw_arguments, bundled_source)?;
    if embedded.is_some() && arguments.options.watch {
        return Err("embedded applications cannot use --watch or --dev-server; use --manifest for development".into());
    }
    arguments.path = std::path::absolute(&arguments.path).map_err(|error| error.to_string())?;
    let _working_directory = root.map(package::WorkingDirectory::enter).transpose()?;
    match (arguments.engine, arguments.check) {
        (Engine::Lua, true) => check_lua_file(&arguments.path)?,
        (Engine::QuickJs, true) => check_js_file(&arguments.path)?,
        (Engine::Lua, false) => return run_lua_file(arguments.path, arguments.options),
        (Engine::QuickJs, false) => return run_js_file(arguments.path, arguments.options),
    }
    println!("{} is valid", arguments.path.display());
    Ok(())
}

#[cfg(test)]
fn parse_arguments(arguments: impl IntoIterator<Item = String>) -> Result<Arguments, String> {
    parse_arguments_with_default(arguments, None)
}

/// The entry of a macOS app bundle `gpx` runs inside, from
/// `Contents/Resources/app`.
fn bundled_source_path(executable: &Path) -> Option<PathBuf> {
    bundled_entries(executable)?
        .into_iter()
        .find(|path| path.is_file())
}

fn bundled_entries(executable: &Path) -> Option<Vec<PathBuf>> {
    let directory = executable.parent()?;
    let contents = directory.parent()?;
    if directory.file_name()? != "MacOS" || contents.file_name()? != "Contents" {
        return None;
    }
    let app = contents.join("Resources/app");
    Some(
        BUNDLED_ENTRIES
            .iter()
            .map(|entry| app.join(entry))
            .collect(),
    )
}

fn engine(path: &Path) -> Result<Engine, String> {
    if is_lua_source(path) {
        Ok(Engine::Lua)
    } else if is_js_source(path) {
        Ok(Engine::QuickJs)
    } else {
        let extensions = LUA_EXTENSIONS.iter().chain(&JS_EXTENSIONS).copied();
        Err(format!(
            "{} isn't an app entry: use a {} file",
            path.display(),
            either(extensions)
        ))
    }
}

/// `.a, .b or .c`.
fn either<'a>(extensions: impl Iterator<Item = &'a str>) -> String {
    let extensions = extensions
        .map(|extension| format!(".{extension}"))
        .collect::<Vec<_>>();
    match extensions.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    }
}

fn parse_arguments_with_default(
    arguments: impl IntoIterator<Item = String>,
    default_path: Option<PathBuf>,
) -> Result<Arguments, String> {
    let mut arguments = arguments.into_iter();
    let mut path = None;
    let mut options = AppOptions::default();
    let mut check = false;

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--check" => check = true,
            "--watch" => options.watch = true,
            "--background" => options.background = true,
            "--dev-server" => {
                options.dev_server = Some(
                    next_value(&mut arguments, "--dev-server")?
                        .parse()
                        .map_err(|_| "--dev-server requires a port from 0 to 65535")?,
                );
                options.watch = true;
            }
            "--title" => options.title = Some(next_value(&mut arguments, "--title")?),
            "--width" => {
                options.width = Some(dimension(next_value(&mut arguments, "--width")?, "width")?);
            }
            "--height" => {
                options.height = Some(dimension(
                    next_value(&mut arguments, "--height")?,
                    "height",
                )?);
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown option {value}\n\n{}", usage()));
            }
            value => {
                if path.replace(PathBuf::from(value)).is_some() {
                    return Err(format!("only one source file is supported\n\n{}", usage()));
                }
            }
        }
    }

    let path = path.or(default_path).ok_or_else(usage)?;
    if check && options.dev_server.is_some() {
        return Err("--check cannot be combined with --dev-server".into());
    }
    Ok(Arguments {
        engine: engine(&path)?,
        path,
        options,
        check,
    })
}

fn next_value(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<String, String> {
    arguments
        .next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn dimension(value: String, name: &str) -> Result<f32, String> {
    let value = value
        .parse::<f32>()
        .map_err(|_| format!("{name} must be a number"))?;
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("{name} must be greater than zero"));
    }
    Ok(value)
}

fn usage() -> String {
    format!(
        "Usage: gpx package gpuix.json [--format app|binary|deb] [--output PATH]\n\
         Usage: gpx [--manifest gpuix.json] [--check] [--watch] [--background] [--dev-server PORT] [--title TITLE] [--width PX] [--height PX] [app]\n\
         The app is a {} file, run in the embedded Lua runtime, or a {} file,\n\
         run in the embedded QuickJS runtime. It is required unless gpx is launched from an app\n\
         bundle with Resources/app/{}, or has an embedded payload or --manifest.\n\
         --check loads the app without opening a window. --watch reloads it when its files\n\
         change. --dev-server enables watching and an authenticated loopback testing API\n\
         (requires GPUIX_DEV_TOKEN). --background opens windows without taking focus.\n\
         --title, --width and --height beat the app's own window options.",
        either(LUA_EXTENSIONS.into_iter()),
        either(JS_EXTENSIONS.into_iter()),
        BUNDLED_ENTRIES.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_launch_is_opt_in_for_both_engines() {
        for entry in ["app.luax", "app.tsx"] {
            let args =
                parse_arguments(["--background", "--dev-server", "0", entry].map(String::from))
                    .unwrap();
            assert!(args.options.background);
            let args = parse_arguments([entry].map(String::from)).unwrap();
            assert!(!args.options.background);
        }
    }

    #[test]
    fn dev_server_enables_watching_and_validates_port() {
        let args = parse_arguments(["--dev-server", "0", "app.luax"].map(String::from)).unwrap();
        assert_eq!(args.options.dev_server, Some(0));
        assert!(args.options.watch);
        for args in [
            vec!["--dev-server", "65536", "app.luax"],
            vec!["--dev-server", "-1", "app.tsx"],
            vec!["--dev-server"],
            vec!["--dev-server", "4318", "--check", "app.lua"],
        ] {
            assert!(parse_arguments(args.into_iter().map(String::from)).is_err());
        }
    }

    #[test]
    fn packaged_runner_uses_the_bundled_source_and_allows_overrides() {
        let entries = bundled_entries(Path::new(
            "/Applications/GPUIX Solid.app/Contents/MacOS/GPUIX Solid",
        ))
        .unwrap();
        assert_eq!(
            entries[0],
            PathBuf::from("/Applications/GPUIX Solid.app/Contents/Resources/app/main.luax")
        );
        assert_eq!(
            entries[2],
            PathBuf::from("/Applications/GPUIX Solid.app/Contents/Resources/app/main.tsx")
        );
        assert!(bundled_entries(Path::new("/usr/local/bin/gpx")).is_none());
        let source = entries[0].clone();
        let arguments =
            parse_arguments_with_default(["--check".into()], Some(source.clone())).unwrap();
        assert_eq!(arguments.path, source);
        assert!(arguments.check);
        let arguments = parse_arguments_with_default(["other.lua".into()], Some(source)).unwrap();
        assert_eq!(arguments.path, PathBuf::from("other.lua"));
        assert!(parse_arguments(Vec::<String>::new()).is_err());
    }

    #[test]
    fn the_entry_extension_picks_the_engine() {
        for (name, expected) in [
            ("app.lua", Engine::Lua),
            ("app.luax", Engine::Lua),
            ("app.tsx", Engine::QuickJs),
            ("app.ts", Engine::QuickJs),
            ("app.mts", Engine::QuickJs),
            ("app.jsx", Engine::QuickJs),
            ("app.js", Engine::QuickJs),
            ("app.mjs", Engine::QuickJs),
            ("APP.LUA", Engine::Lua),
        ] {
            assert_eq!(
                parse_arguments([name.to_string()]).unwrap().engine,
                expected
            );
        }
        for name in ["app.py", "app", "app.d", "app.TSX"] {
            let error = parse_arguments([name.to_string()]).err().unwrap();
            assert!(error.contains("isn't an app entry"), "{error}");
        }
        let error = parse_arguments(["app.py".to_string()]).err().unwrap();
        assert!(
            error.ends_with("use a .lua, .luax, .tsx, .ts, .mts, .jsx, .js or .mjs file"),
            "{error}"
        );
    }

    #[test]
    fn parses_window_options_and_source() {
        let arguments = parse_arguments([
            "--title".to_string(),
            "Counter".to_string(),
            "--width".to_string(),
            "640".to_string(),
            "app.luax".to_string(),
        ])
        .unwrap();
        assert_eq!(arguments.path, PathBuf::from("app.luax"));
        assert_eq!(arguments.options.title.as_deref(), Some("Counter"));
        assert_eq!(arguments.options.width, Some(640.0));
        assert_eq!(arguments.options.height, None);
        assert!(!arguments.options.watch);
    }

    #[test]
    fn parses_watch_mode() {
        let arguments = parse_arguments(["--watch".to_string(), "app.tsx".to_string()]).unwrap();
        assert!(arguments.options.watch);
    }

    #[test]
    fn trays_are_defined_by_the_application_not_a_launcher_flag() {
        assert!(parse_arguments(["--tray", "icon.svg", "app.lua"].map(String::from)).is_err());
    }

    #[test]
    fn rejects_non_positive_dimensions() {
        let error = parse_arguments([
            "--height".to_string(),
            "0".to_string(),
            "app.lua".to_string(),
        ])
        .err()
        .unwrap();
        assert_eq!(error, "height must be greater than zero");
    }
}
