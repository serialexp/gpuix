use std::path::{Path, PathBuf};

use gpuix_native::{check_lua_file, run_lua_file, LuaAppOptions};

struct Arguments {
    path: PathBuf,
    options: LuaAppOptions,
    check: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("gpuix-lua: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let raw_arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if raw_arguments
        .iter()
        .any(|argument| argument == "-h" || argument == "--help")
    {
        println!("{}", usage());
        return Ok(());
    }
    let bundled_source = std::env::current_exe()
        .ok()
        .and_then(|path| bundled_source_path(&path))
        .filter(|path| path.is_file());
    let arguments = parse_arguments_with_default(raw_arguments, bundled_source)?;
    if arguments.check {
        check_lua_file(&arguments.path)?;
        println!("{} is valid", arguments.path.display());
        return Ok(());
    }
    run_lua_file(arguments.path, arguments.options)
}

#[cfg(test)]
fn parse_arguments(arguments: impl IntoIterator<Item = String>) -> Result<Arguments, String> {
    parse_arguments_with_default(arguments, None)
}

fn bundled_source_path(executable: &Path) -> Option<PathBuf> {
    let directory = executable.parent()?;
    let contents = directory.parent()?;
    if directory.file_name()? != "MacOS" || contents.file_name()? != "Contents" {
        return None;
    }
    Some(contents.join("Resources/app/main.luax"))
}

fn parse_arguments_with_default(
    arguments: impl IntoIterator<Item = String>,
    default_path: Option<PathBuf>,
) -> Result<Arguments, String> {
    let mut arguments = arguments.into_iter();
    let mut path = None;
    let mut options = LuaAppOptions::default();
    let mut check = false;

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--check" => check = true,
            "--watch" => options.watch = true,
            "--title" => options.title = Some(next_value(&mut arguments, "--title")?),
            "--width" => {
                options.width = dimension(next_value(&mut arguments, "--width")?, "width")?;
            }
            "--height" => {
                options.height = dimension(next_value(&mut arguments, "--height")?, "height")?;
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

    let path = path.or(default_path).ok_or_else(|| usage().to_string())?;
    Ok(Arguments {
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

fn usage() -> &'static str {
    "Usage: gpuix-lua [--check] [--watch] [--title TITLE] [--width PX] [--height PX] [app.lua|app.luax]\nA source file is required unless launched from an app bundle containing Resources/app/main.luax."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_runner_uses_the_bundled_source_and_allows_overrides() {
        let source = bundled_source_path(Path::new(
            "/Applications/GPUIX Solid.app/Contents/MacOS/GPUIX Solid",
        ))
        .unwrap();
        assert_eq!(
            source,
            PathBuf::from("/Applications/GPUIX Solid.app/Contents/Resources/app/main.luax")
        );
        assert!(bundled_source_path(Path::new("/usr/local/bin/gpuix-lua")).is_none());
        let arguments =
            parse_arguments_with_default(["--check".into()], Some(source.clone())).unwrap();
        assert_eq!(arguments.path, source);
        assert!(arguments.check);
        let arguments = parse_arguments_with_default(["other.lua".into()], Some(source)).unwrap();
        assert_eq!(arguments.path, PathBuf::from("other.lua"));
        assert!(parse_arguments(Vec::<String>::new()).is_err());
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
        assert_eq!(arguments.options.width, 640.0);
        assert!(!arguments.options.watch);
    }

    #[test]
    fn parses_watch_mode() {
        let arguments = parse_arguments(["--watch".to_string(), "app.luax".to_string()]).unwrap();
        assert!(arguments.options.watch);
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
