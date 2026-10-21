use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Payload {
    pub format_version: u32,
    pub manifest: Manifest,
    pub files: Vec<PayloadFile>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PayloadFile {
    pub path: String,
    pub data: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Manifest {
    pub schema_version: u32,
    pub name: String,
    pub identifier: String,
    pub version: String,
    pub entry: String,
    pub files: Vec<String>,
    pub icon: Option<String>,
    pub macos: Option<MacOs>,
    pub linux: Option<Linux>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Linux {
    pub package_name: Option<String>,
    pub maintainer: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub categories: Option<Vec<String>>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct MacOs {
    pub minimum_system_version: Option<String>,
    pub category: Option<String>,
}

impl Manifest {
    pub fn read(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let manifest: Self = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("unsupported gpuix.json schemaVersion; expected 1".into());
        }
        if self.name.trim().is_empty()
            || self.name.contains(['/', '\\', '\0'])
            || self.identifier.is_empty()
            || self.version.is_empty()
            || self.files.is_empty()
        {
            return Err("manifest requires name, identifier, version and files".into());
        }
        let identifier_parts = self.identifier.split('.').collect::<Vec<_>>();
        if identifier_parts.len() < 2
            || identifier_parts.iter().any(|part| {
                part.is_empty()
                    || !part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
        {
            return Err("expected a reverse-domain identifier".into());
        }
        let version_parts = self.version.split('.').collect::<Vec<_>>();
        if version_parts.len() != 3
            || version_parts
                .iter()
                .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Err("expected a numeric major.minor.patch version".into());
        }
        relative_path(&self.entry)?;
        super::engine(Path::new(&self.entry))?;
        for file in &self.files {
            if file != "." {
                relative_path(file)?;
            }
        }
        if let Some(icon) = &self.icon {
            if !icon.ends_with(".icns") {
                return Err("manifest icon must name a macOS .icns file".into());
            }
        }
        if let Some(macos) = &self.macos {
            if macos.minimum_system_version.as_deref() == Some("")
                || macos.category.as_deref() == Some("")
            {
                return Err("macOS options must not be empty".into());
            }
            if let Some(minimum) = &macos.minimum_system_version {
                let parts = minimum.split('.').collect::<Vec<_>>();
                if parts.len() > 3
                    || parts.iter().any(|part| {
                        part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit())
                    })
                    || parts[0].parse::<u64>().map_or(true, |major| major < 14)
                {
                    return Err("expected macOS minimumSystemVersion of 14 or later".into());
                }
            }
        }
        if let Some(linux) = &self.linux {
            for value in [
                &linux.package_name,
                &linux.maintainer,
                &linux.description,
                &linux.icon,
            ]
            .into_iter()
            .flatten()
            {
                if value.trim().is_empty() || value.contains(['\r', '\n', '\0']) {
                    return Err("Linux metadata must contain nonempty single-line values".into());
                }
            }
            if let Some(categories) = &linux.categories {
                if categories.is_empty()
                    || categories.iter().any(|category| {
                        category.is_empty()
                            || !category
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                    })
                {
                    return Err("invalid Linux desktop categories".into());
                }
            }
            if let Some(name) = &linux.package_name {
                if name.len() < 2
                    || !name.as_bytes()[0].is_ascii_lowercase()
                        && !name.as_bytes()[0].is_ascii_digit()
                    || !name.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"+.-".contains(&byte)
                    })
                {
                    return Err("invalid Debian packageName".into());
                }
            }
            if let Some(icon) = &linux.icon {
                if !icon.ends_with(".svg") {
                    return Err("linux.icon must name an SVG file".into());
                }
            }
            if let Some(maintainer) = &linux.maintainer {
                let Some((name, email)) = maintainer.split_once(" <") else {
                    return Err("expected linux.maintainer as Name <email>".into());
                };
                if name.trim().is_empty()
                    || !email.ends_with('>')
                    || !email.contains('@')
                    || email[..email.len() - 1]
                        .chars()
                        .any(|value| value.is_whitespace() || value == '<' || value == '>')
                {
                    return Err("expected linux.maintainer as Name <email>".into());
                }
            }
        }
        Ok(())
    }
}

pub(super) fn relative_path(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains(['\\', '\0', ':'])
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!("invalid application-relative path {value:?}"));
    }
    Ok(path.to_path_buf())
}

pub(super) struct EmbeddedApp {
    directory: tempfile::TempDir,
    pub manifest: Manifest,
}

pub(super) struct WorkingDirectory(PathBuf);

impl WorkingDirectory {
    pub fn enter(path: &Path) -> Result<Self, String> {
        let previous = std::env::current_dir().map_err(|error| error.to_string())?;
        std::env::set_current_dir(path).map_err(|error| error.to_string())?;
        Ok(Self(previous))
    }
}

impl Drop for WorkingDirectory {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}

impl EmbeddedApp {
    pub fn load() -> Result<Option<Self>, String> {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        super::archive::read(&executable)?
            .map(|bytes| Self::extract(&bytes))
            .transpose()
    }

    fn extract(bytes: &[u8]) -> Result<Self, String> {
        let payload: Payload = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        if payload.format_version != 1 {
            return Err("unsupported embedded application format".into());
        }
        payload.manifest.validate()?;
        let directory = tempfile::Builder::new()
            .prefix("gpuix-app-")
            .tempdir()
            .map_err(|error| error.to_string())?;
        let mut seen = HashSet::new();
        for file in payload.files {
            let relative = relative_path(&file.path)?;
            if !seen.insert(relative.clone()) {
                return Err(format!("duplicate embedded file {}", file.path));
            }
            let path = directory.path().join(relative);
            let data = base64::engine::general_purpose::STANDARD
                .decode(file.data)
                .map_err(|error| error.to_string())?;
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|error| error.to_string())?;
            std::fs::write(path, data).map_err(|error| error.to_string())?;
        }
        if !directory.path().join(&payload.manifest.entry).is_file() {
            return Err("embedded application entry is missing".into());
        }
        Ok(Self {
            directory,
            manifest: payload.manifest,
        })
    }

    pub fn root(&self) -> &Path {
        self.directory.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(files: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "formatVersion": 1,
            "manifest": {
                "schemaVersion": 1, "name": "Test", "identifier": "dev.gpuix.test",
                "version": "0.1.0", "entry": "main.lua", "files": ["main.lua"]
            },
            "files": files
        }))
        .unwrap()
    }

    #[test]
    fn extracts_binary_assets_and_cleans_up() {
        let app = EmbeddedApp::extract(&payload(serde_json::json!([
            {"path": "main.lua", "data": "cmV0dXJuIDE="},
            {"path": "assets/raw.bin", "data": "AP8="}
        ])))
        .unwrap();
        let root = app.root().to_path_buf();
        assert_eq!(
            std::fs::read(root.join("assets/raw.bin")).unwrap(),
            [0, 255]
        );
        drop(app);
        assert!(!root.exists());
    }

    #[test]
    fn rejects_unsafe_duplicate_or_missing_entries() {
        for path in ["../escape", "/absolute", "a/../../escape", "C:\\escape"] {
            assert!(EmbeddedApp::extract(&payload(serde_json::json!([
                {"path": path, "data": ""}
            ])))
            .is_err());
        }
        assert!(EmbeddedApp::extract(&payload(serde_json::json!([]))).is_err());
        assert!(EmbeddedApp::extract(&payload(serde_json::json!([
            {"path": "main.lua", "data": ""}, {"path": "main.lua", "data": ""}
        ])))
        .is_err());
    }

    #[test]
    fn rejects_unknown_manifest_fields_and_invalid_identity() {
        let original: serde_json::Value = serde_json::from_slice(&payload(serde_json::json!([
            {"path": "main.lua", "data": ""}
        ])))
        .unwrap();
        assert!(EmbeddedApp::extract(&serde_json::to_vec(&original).unwrap()).is_ok());
        for (key, value) in [
            ("permissions", serde_json::json!({})),
            ("schemaVersion", serde_json::json!(2)),
            ("identifier", serde_json::json!("not a bundle id")),
            ("version", serde_json::json!("next")),
            ("entry", serde_json::json!("../main.lua")),
            ("macos", serde_json::json!({"minimumSystemVersion": "13.0"})),
            ("linux", serde_json::json!({"packageName": "../escape"})),
            ("linux", serde_json::json!({"maintainer": "not an email"})),
            (
                "linux",
                serde_json::json!({"categories": ["Utility;Exec=bad"]}),
            ),
            ("linux", serde_json::json!({"icon": "app.icns"})),
        ] {
            let mut invalid = original.clone();
            invalid["manifest"][key] = value;
            assert!(EmbeddedApp::extract(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
    }

    #[test]
    fn accepts_linux_distribution_metadata() {
        let mut value: serde_json::Value = serde_json::from_slice(&payload(serde_json::json!([
            {"path": "main.lua", "data": ""}
        ])))
        .unwrap();
        value["manifest"]["linux"] = serde_json::json!({
            "packageName": "example-app", "maintainer": "Example <dev@example.com>",
            "description": "Example application", "icon": "assets/app.svg",
            "categories": ["Utility"]
        });
        assert!(EmbeddedApp::extract(&serde_json::to_vec(&value).unwrap()).is_ok());
        value["manifest"]["linux"]["unknown"] = serde_json::json!(true);
        assert!(EmbeddedApp::extract(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}
