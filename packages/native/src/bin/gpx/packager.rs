use std::path::{Path, PathBuf};
use std::process::Command;

pub(super) const USAGE: &str = "Usage: gpx package gpuix.json [--format app|binary|deb] [--output PATH]\nBuild a native application using the GPUIX source checkout, Bun and Rust.\nSet GPUIX_SOURCE_DIR if the checkout cannot be found automatically.";

fn validate(arguments: &[String]) -> Result<(), String> {
    if arguments.is_empty() || arguments[0].starts_with('-') {
        return Err(USAGE.into());
    }
    let mut options = arguments[1..].chunks_exact(2);
    for option in &mut options {
        match option[0].as_str() {
            "--format" if ["app", "binary", "deb"].contains(&option[1].as_str()) => {}
            "--format" => return Err("--format must be app, binary or deb".into()),
            "--output" if !option[1].is_empty() => {}
            _ => return Err(format!("unknown package option {}\n{USAGE}", option[0])),
        }
    }
    if !options.remainder().is_empty() {
        return Err(format!("{} requires a value", options.remainder()[0]));
    }
    Ok(())
}

fn is_checkout(path: &Path) -> bool {
    path.join("scripts/app.ts").is_file() && path.join("packages/native/Cargo.toml").is_file()
}

fn source_directory(current: &Path) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("GPUIX_SOURCE_DIR") {
        let path = std::fs::canonicalize(path).map_err(|error| error.to_string())?;
        return if is_checkout(&path) {
            Ok(path)
        } else {
            Err("GPUIX_SOURCE_DIR is not a GPUIX source checkout".into())
        };
    }
    current.ancestors()
        .chain(Path::new(env!("CARGO_MANIFEST_DIR")).ancestors())
        .find(|path| is_checkout(path))
        .map(Path::to_path_buf)
        .ok_or_else(|| "Packaging currently requires a GPUIX source checkout. Set GPUIX_SOURCE_DIR to its location.".into())
}

pub(super) fn run(arguments: &[String]) -> Result<(), String> {
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        println!("{USAGE}");
        return Ok(());
    }
    validate(arguments)?;
    let current = std::env::current_dir().map_err(|error| error.to_string())?;
    let source = source_directory(&current)?;
    let status = Command::new("bun")
        .arg(source.join("scripts/app.ts"))
        .arg("package")
        .args(arguments)
        .env("GPUIX_PACKAGE_OUTPUT_ROOT", current)
        .env_remove("GPUIX_APP_PAYLOAD")
        .status()
        .map_err(|error| {
            format!("cannot start packaging: {error}; install Bun and the Rust build toolchain")
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("packaging failed ({status})"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_packaging_arguments() {
        for arguments in [
            vec!["gpuix.json"],
            vec![
                "some app/gpuix.json",
                "--format",
                "deb",
                "--output",
                "some output/app.deb",
            ],
        ] {
            assert!(validate(&arguments.into_iter().map(String::from).collect::<Vec<_>>()).is_ok());
        }
        for arguments in [
            vec![],
            vec!["--format", "deb"],
            vec!["gpuix.json", "--format"],
            vec!["gpuix.json", "--format", "db"],
            vec!["gpuix.json", "--unknown", "value"],
        ] {
            assert!(
                validate(&arguments.into_iter().map(String::from).collect::<Vec<_>>()).is_err()
            );
        }
    }

    #[test]
    fn locates_the_build_tooling_checkout() {
        assert!(Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .any(is_checkout));
    }
}
