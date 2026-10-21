use std::path::Path;

use super::{command, set_mode, Manifest, LINUX_ICON};

fn architecture() -> Result<&'static str, String> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("amd64"),
        "aarch64" => Ok("arm64"),
        _ => Err("Debian packaging supports amd64 and arm64".into()),
    }
}

fn name(manifest: &Manifest) -> String {
    manifest
        .linux
        .as_ref()
        .and_then(|linux| linux.package_name.clone())
        .unwrap_or_else(|| manifest.identifier.to_lowercase())
}

pub(super) fn filename(manifest: &Manifest) -> Result<String, String> {
    if manifest
        .linux
        .as_ref()
        .and_then(|linux| linux.maintainer.as_ref())
        .is_none()
    {
        return Err("Debian packaging requires linux.maintainer: Name <email>".into());
    }
    Ok(format!(
        "{}_{}_{}.deb",
        name(manifest),
        manifest.version,
        architecture()?
    ))
}

fn metadata(
    manifest: &Manifest,
    architecture: &str,
    dependencies: &str,
) -> Result<(String, String), String> {
    let name = name(manifest);
    let linux = manifest
        .linux
        .as_ref()
        .ok_or("Debian packaging requires linux.maintainer")?;
    let maintainer = linux
        .maintainer
        .as_deref()
        .ok_or("Debian packaging requires linux.maintainer")?;
    let description = linux.description.as_deref().unwrap_or(&manifest.name);
    if description.contains(['\n', '\r', '\0']) {
        return Err("Debian description must be a single line".into());
    }
    let control = format!("Package: {name}\nVersion: {}\nSection: utils\nPriority: optional\nArchitecture: {architecture}\nMaintainer: {maintainer}\nDepends: {dependencies}\nDescription: {description}\n", manifest.version);
    let escape = |value: &str| {
        value
            .replace('\\', "\\\\")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
    };
    let categories = linux
        .categories
        .as_ref()
        .map(|categories| categories.join(";"))
        .unwrap_or_else(|| "Utility".into());
    let desktop = format!("[Desktop Entry]\nType=Application\nName={}\nComment={}\nExec=/usr/bin/{name}\nTryExec=/usr/bin/{name}\nIcon={name}\nTerminal=false\nCategories={categories};\n", escape(&manifest.name), escape(description));
    Ok((control, desktop))
}

pub(super) fn build(
    manifest: &Manifest,
    root: &Path,
    binary: &Path,
    artifact: &Path,
    stage: &Path,
) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("Debian packaging requires Linux".into());
    }
    let architecture = architecture()?;
    let native = command("dpkg", &["--print-architecture".as_ref()], stage)?;
    if native != architecture {
        return Err("Debian host architecture does not match this gpx runner".into());
    }
    let name = name(manifest);
    let (initial, desktop) = metadata(manifest, architecture, "pending")?;
    let tree = stage.join("debian-package");
    for path in [
        "DEBIAN",
        "usr/bin",
        "usr/share/applications",
        "usr/share/icons/hicolor/scalable/apps",
    ] {
        let directory = tree.join(path);
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        for ancestor in directory
            .ancestors()
            .take_while(|ancestor| *ancestor != stage)
        {
            set_mode(ancestor, 0o755)?;
        }
    }
    let installed = tree.join("usr/bin").join(&name);
    std::fs::copy(binary, &installed).map_err(|error| error.to_string())?;
    let icon = manifest
        .linux
        .as_ref()
        .and_then(|linux| linux.icon.as_ref())
        .map(|icon| std::fs::read(root.join(icon)))
        .transpose()
        .map_err(|error| error.to_string())?;
    std::fs::write(
        tree.join("usr/share/icons/hicolor/scalable/apps")
            .join(format!("{name}.svg")),
        icon.as_deref().unwrap_or(LINUX_ICON),
    )
    .map_err(|error| error.to_string())?;
    let desktop_path = tree
        .join("usr/share/applications")
        .join(format!("{name}.desktop"));
    std::fs::write(&desktop_path, desktop).map_err(|error| error.to_string())?;
    set_mode(&desktop_path, 0o644)?;
    set_mode(
        &tree
            .join("usr/share/icons/hicolor/scalable/apps")
            .join(format!("{name}.svg")),
        0o644,
    )?;
    command("desktop-file-validate", &[desktop_path.as_os_str()], stage)?;
    std::fs::write(tree.join("DEBIAN/control"), &initial).map_err(|error| error.to_string())?;
    std::fs::create_dir(stage.join("debian")).map_err(|error| error.to_string())?;
    std::fs::write(stage.join("debian/control"), format!("Source: {name}\nSection: utils\nPriority: optional\nMaintainer: {}\nStandards-Version: 4.6.2\n\n{initial}", manifest.linux.as_ref().unwrap().maintainer.as_ref().unwrap())).map_err(|error| error.to_string())?;
    let detected = command(
        "dpkg-shlibdeps",
        &["-O".as_ref(), installed.as_os_str()],
        stage,
    )?;
    let linked = detected
        .lines()
        .find_map(|line| line.strip_prefix("shlibs:Depends="))
        .ok_or("dpkg-shlibdeps did not report dependencies")?;
    let runtime = [
        "hicolor-icon-theme",
        "libvulkan1",
        "libwayland-client0",
        "libwayland-cursor0",
        "libwayland-egl1",
        "libxkbcommon0",
        "libxkbcommon-x11-0",
        "libx11-6",
        "libxcb1",
        "libfontconfig1",
        "libfreetype6",
    ];
    let mut dependencies = linked.to_string();
    for dependency in runtime {
        if !linked
            .split(',')
            .any(|linked| linked.split_whitespace().next() == Some(dependency))
        {
            dependencies.push_str(&format!(", {dependency}"));
        }
    }
    std::fs::write(
        tree.join("DEBIAN/control"),
        metadata(manifest, architecture, &dependencies)?.0,
    )
    .map_err(|error| error.to_string())?;
    set_mode(&tree.join("DEBIAN/control"), 0o644)?;
    command(
        "dpkg-deb",
        &[
            "--root-owner-group".as_ref(),
            "--build".as_ref(),
            tree.as_os_str(),
            artifact.as_os_str(),
        ],
        stage,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_control_and_shell_free_desktop_entries() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("gpuix.json");
        std::fs::write(&path, r#"{"schemaVersion":1,"name":"My App","identifier":"dev.example.app","version":"1.2.3","entry":"main.lua","files":["main.lua"],"linux":{"maintainer":"Example <dev@example.com>","packageName":"my-app","categories":["Development"],"description":"An example"}}"#).unwrap();
        let manifest = Manifest::read(&path).unwrap();
        let (control, desktop) = metadata(&manifest, "amd64", "libc6 (>= 2.39)").unwrap();
        assert!(control.contains("Package: my-app\n"));
        assert!(control.contains("Depends: libc6 (>= 2.39)\n"));
        assert!(desktop.contains("Exec=/usr/bin/my-app\n"));
        assert!(desktop.contains("Categories=Development;\n"));
    }
}
