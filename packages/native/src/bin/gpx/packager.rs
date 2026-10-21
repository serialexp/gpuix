use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine as _;

use super::package::{relative_path, Manifest, Payload, PayloadFile};

#[path = "debian.rs"]
mod debian;

pub(super) const USAGE: &str = "Usage: gpx package gpuix.json [--format app|binary|deb] [--output PATH]\nPackage with the installed native runner; no checkout, Bun, Rust or compiler required.\nBuilds for the runner's OS and architecture. macOS signing requires codesign; .deb requires Debian packaging tools.";
const APP_ICON: &[u8] = include_bytes!("../../../../../scripts/assets/AppIcon.icns");
const LINUX_ICON: &[u8] = include_bytes!("../../../../../scripts/assets/app-icon.svg");

struct Options {
    manifest: PathBuf,
    format: String,
    output: Option<PathBuf>,
}

fn options(arguments: &[String]) -> Result<Options, String> {
    if arguments.is_empty() || arguments[0].starts_with('-') {
        return Err(USAGE.into());
    }
    let mut result = Options {
        manifest: PathBuf::from(&arguments[0]),
        format: if cfg!(target_os = "macos") {
            "app"
        } else {
            "binary"
        }
        .into(),
        output: None,
    };
    let mut pairs = arguments[1..].chunks_exact(2);
    for pair in &mut pairs {
        match pair[0].as_str() {
            "--format" if ["app", "binary", "deb"].contains(&pair[1].as_str()) => {
                result.format = pair[1].clone()
            }
            "--format" => return Err("--format must be app, binary or deb".into()),
            "--output" if !pair[1].is_empty() => result.output = Some(PathBuf::from(&pair[1])),
            _ => return Err(format!("unknown package option {}\n{USAGE}", pair[0])),
        }
    }
    if !pairs.remainder().is_empty() {
        return Err(format!("{} requires a value", pairs.remainder()[0]));
    }
    if result.format == "app" && !cfg!(target_os = "macos") {
        return Err("app bundles require macOS".into());
    }
    if result.format == "deb" && !cfg!(target_os = "linux") {
        return Err("Debian packages require a native Debian/Ubuntu Linux host".into());
    }
    Ok(result)
}

fn collect(root: &Path, includes: &[String], output: &Path) -> Result<Vec<PayloadFile>, String> {
    fn is_link(metadata: &std::fs::Metadata) -> bool {
        if metadata.file_type().is_symlink() {
            return true;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            return metadata.file_attributes() & 0x400 != 0;
        }
        #[cfg(not(windows))]
        false
    }
    fn visit(
        root: &Path,
        path: &Path,
        output: &Path,
        files: &mut BTreeMap<String, String>,
        size: &mut u64,
    ) -> Result<(), String> {
        if path == output {
            return Err("package output is included in the payload; use an explicit files list or another output directory".into());
        }
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if is_link(&metadata) {
            return Err(format!(
                "payload symlinks are not supported: {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            for child in std::fs::read_dir(path).map_err(|error| error.to_string())? {
                visit(
                    root,
                    &child.map_err(|error| error.to_string())?.path(),
                    output,
                    files,
                    size,
                )?;
            }
        } else if metadata.is_file() {
            let name = path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_str()
                .ok_or("payload paths must be UTF-8")?
                .replace('\\', "/");
            if files.contains_key(&name) {
                return Ok(());
            }
            *size = size
                .checked_add(metadata.len())
                .ok_or("payload size overflow")?;
            if *size > 384 * 1024 * 1024 {
                return Err(
                    "application files exceed 384 MiB (512 MiB encoded payload limit)".into(),
                );
            }
            let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
            files.insert(
                name,
                base64::engine::general_purpose::STANDARD.encode(bytes),
            );
        } else {
            return Err(format!("unsupported payload file {}", path.display()));
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    let mut size = 0;
    for include in includes {
        if include != "." {
            relative_path(include)?;
        }
        let mut path = root.to_path_buf();
        for part in Path::new(include)
            .components()
            .filter(|part| *part != std::path::Component::CurDir)
        {
            path.push(part);
            if is_link(&std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?) {
                return Err(format!(
                    "payload symlinks are not supported: {}",
                    path.display()
                ));
            }
        }
        visit(root, &path, output, &mut files, &mut size)?;
    }
    Ok(files
        .into_iter()
        .map(|(path, data)| PayloadFile { path, data })
        .collect())
}

pub(super) fn command(
    program: &str,
    arguments: &[&std::ffi::OsStr],
    directory: &Path,
) -> Result<String, String> {
    let output = Command::new(program)
        .args(arguments)
        .current_dir(directory)
        .output()
        .map_err(|error| format!("cannot run {program}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn write_runner(source: &Path, destination: &Path, payload: &[u8]) -> Result<(), String> {
    std::fs::copy(source, destination).map_err(|error| error.to_string())?;
    set_mode(destination, 0o755)?;
    #[cfg(target_os = "macos")]
    if super::archive::signed(destination)? {
        command(
            "codesign",
            &["--remove-signature".as_ref(), destination.as_os_str()],
            destination.parent().unwrap(),
        )?;
    }
    super::archive::append(destination, payload)
}

pub(super) fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    let mut permissions = std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(mode);
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        permissions.set_readonly(false);
    }
    std::fs::set_permissions(path, permissions).map_err(|error| error.to_string())
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn app_bundle(
    path: &Path,
    manifest: &Manifest,
    root: &Path,
    source: &Path,
    payload: &[u8],
) -> Result<PathBuf, String> {
    let contents = path.join("Contents");
    std::fs::create_dir_all(contents.join("MacOS")).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(contents.join("Resources")).map_err(|error| error.to_string())?;
    for directory in [
        path.to_path_buf(),
        contents.clone(),
        contents.join("MacOS"),
        contents.join("Resources"),
    ] {
        set_mode(&directory, 0o755)?;
    }
    let executable = contents.join("MacOS").join(&manifest.name);
    write_runner(source, &executable, payload)?;
    let icon = manifest
        .icon
        .as_ref()
        .map(|icon| std::fs::read(root.join(icon)))
        .transpose()
        .map_err(|error| error.to_string())?;
    std::fs::write(
        contents.join("Resources/AppIcon.icns"),
        icon.as_deref().unwrap_or(APP_ICON),
    )
    .map_err(|error| error.to_string())?;
    set_mode(&contents.join("Resources/AppIcon.icns"), 0o644)?;
    let fields = [
        ("CFBundleExecutable", manifest.name.as_str()),
        ("CFBundleIdentifier", manifest.identifier.as_str()),
        ("CFBundleName", manifest.name.as_str()),
        ("CFBundleDisplayName", manifest.name.as_str()),
        ("CFBundleIconFile", "AppIcon.icns"),
        ("CFBundlePackageType", "APPL"),
        ("CFBundleShortVersionString", manifest.version.as_str()),
        ("CFBundleVersion", manifest.version.as_str()),
        (
            "LSMinimumSystemVersion",
            manifest
                .macos
                .as_ref()
                .and_then(|macos| macos.minimum_system_version.as_deref())
                .unwrap_or("14.0"),
        ),
        (
            "LSApplicationCategoryType",
            manifest
                .macos
                .as_ref()
                .and_then(|macos| macos.category.as_deref())
                .unwrap_or("public.app-category.developer-tools"),
        ),
    ];
    let mut plist = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"><plist version=\"1.0\"><dict>".to_string();
    for (key, value) in fields {
        plist.push_str(&format!("<key>{key}</key><string>{}</string>", xml(value)));
    }
    plist.push_str("<key>NSHighResolutionCapable</key><true/></dict></plist>");
    std::fs::write(contents.join("Info.plist"), plist).map_err(|error| error.to_string())?;
    set_mode(&contents.join("Info.plist"), 0o644)?;
    Ok(executable)
}

fn publish(staged: &Path, output: &Path, stage: &Path) -> Result<(), String> {
    let backup = stage.join("previous-output");
    let exists = output.symlink_metadata().is_ok();
    if exists {
        std::fs::rename(output, &backup).map_err(|error| error.to_string())?;
    }
    if let Err(error) = std::fs::rename(staged, output) {
        if exists {
            let _ = std::fs::rename(&backup, output);
        }
        return Err(error.to_string());
    }
    Ok(())
}

fn check(executable: &Path, directory: &Path) -> Result<(), String> {
    let mut child = Command::new(executable)
        .arg("--check")
        .current_dir(directory)
        .stdin(std::process::Stdio::null())
        .env_remove("LUA_PATH")
        .env_remove("LUA_CPATH")
        .env_remove("NODE_PATH")
        .spawn()
        .map_err(|error| format!("cannot validate packaged app: {error}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("packaged application failed --check: {status}"))
            };
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("packaged application --check exceeded 60 seconds".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

pub(super) fn run(arguments: &[String]) -> Result<(), String> {
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        println!("{USAGE}");
        return Ok(());
    }
    let options = options(arguments)?;
    let manifest_path =
        std::fs::canonicalize(options.manifest).map_err(|error| error.to_string())?;
    let root = manifest_path.parent().unwrap();
    let manifest = Manifest::read(&manifest_path)?;
    super::engine(Path::new(&manifest.entry))?;
    let name = match options.format.as_str() {
        "app" => format!("{}.app", manifest.name),
        "deb" => debian::filename(&manifest)?,
        _ => format!(
            "{}{}",
            manifest.name,
            if cfg!(target_os = "windows") {
                ".exe"
            } else {
                ""
            }
        ),
    };
    let output = options
        .output
        .unwrap_or_else(|| PathBuf::from("dist").join(name));
    if options.format == "app"
        && output
            .extension()
            .is_none_or(|extension| extension != "app")
    {
        return Err("app output must end with .app".into());
    }
    if options.format == "deb"
        && output
            .extension()
            .is_none_or(|extension| extension != "deb")
    {
        return Err("Debian output must end with .deb".into());
    }
    let absolute = std::path::absolute(&output).map_err(|error| error.to_string())?;
    let parent = absolute.parent().ok_or("output needs a parent directory")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let output = std::fs::canonicalize(parent)
        .map_err(|error| error.to_string())?
        .join(absolute.file_name().ok_or("invalid output filename")?);
    let source = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|error| error.to_string())?;
    let target = std::fs::canonicalize(&output).unwrap_or_else(|_| output.clone());
    let current = std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .map_err(|error| error.to_string())?;
    if source.starts_with(&target)
        || target == manifest_path
        || root.starts_with(&target)
        || current.starts_with(&target)
    {
        return Err(
            "output must not replace the runner, manifest, working directory or application source directory".into(),
        );
    }
    if let Ok(metadata) = std::fs::metadata(&output) {
        if metadata.is_dir() != (options.format == "app") {
            return Err("output type conflicts with an existing file or directory".into());
        }
    }
    let files = collect(root, &manifest.files, &target)?;
    if !files.iter().any(|file| file.path == manifest.entry) {
        return Err("files list does not include the application entry".into());
    }
    let data = Payload {
        format_version: 1,
        manifest,
        files,
    };
    let payload = serde_json::to_vec(&data).map_err(|error| error.to_string())?;
    let manifest = &data.manifest;
    let stage = tempfile::Builder::new()
        .prefix(".gpuix-package-")
        .tempdir_in(output.parent().unwrap())
        .map_err(|error| error.to_string())?;
    let artifact = stage.path().join(output.file_name().unwrap());
    let executable = if options.format == "app" {
        app_bundle(&artifact, manifest, root, &source, &payload)?
    } else {
        let executable = if options.format == "deb" {
            stage.path().join("runner")
        } else {
            artifact.clone()
        };
        write_runner(&source, &executable, &payload)?;
        executable
    };
    #[cfg(target_os = "macos")]
    {
        let signing_target = if options.format == "app" {
            &artifact
        } else {
            &executable
        };
        command(
            "codesign",
            &[
                "--force".as_ref(),
                "--sign".as_ref(),
                "-".as_ref(),
                signing_target.as_os_str(),
            ],
            stage.path(),
        )?;
        command(
            "codesign",
            &[
                "--verify".as_ref(),
                "--strict".as_ref(),
                signing_target.as_os_str(),
            ],
            stage.path(),
        )?;
    }
    check(&executable, stage.path())?;
    if options.format == "deb" {
        debian::build(manifest, root, &executable, &artifact, stage.path())?;
    }
    publish(&artifact, &output, stage.path())?;
    println!("Packaged: {}", output.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_packaging_arguments() {
        assert!(options(&["gpuix.json".into(), "--format".into(), "binary".into()]).is_ok());
        for args in [
            vec![],
            vec!["app.json", "--format"],
            vec!["app.json", "--format", "db"],
            vec!["app.json", "--bad", "value"],
        ] {
            assert!(options(&args.into_iter().map(String::from).collect::<Vec<_>>()).is_err());
        }
    }

    #[test]
    fn collects_nested_files_and_rejects_output_inclusion() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("assets")).unwrap();
        std::fs::write(root.path().join("assets/raw"), [0, 255]).unwrap();
        let files = collect(
            root.path(),
            &["assets".into(), "assets/raw".into()],
            &root.path().join("output"),
        )
        .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].data, "AP8=");
        let all = collect(root.path(), &[".".into()], &root.path().join("output")).unwrap();
        assert_eq!(all[0].path, "assets/raw");
        assert!(collect(root.path(), &[".".into()], &root.path().join("assets/raw")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_files_and_parents() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("real")).unwrap();
        std::fs::write(root.path().join("real/file"), "value").unwrap();
        std::os::unix::fs::symlink(root.path().join("real"), root.path().join("link")).unwrap();
        assert!(collect(
            root.path(),
            &["link/file".into()],
            &root.path().join("output")
        )
        .is_err());
    }

    #[test]
    fn escapes_bundle_metadata() {
        assert_eq!(xml("<&\"'>"), "&lt;&amp;&quot;&apos;&gt;");
    }
}
