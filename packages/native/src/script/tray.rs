use std::path::Path;

pub(crate) fn validate(path: Option<&Path>) -> Result<(), String> {
    let Some(path) = path else { return Ok(()) };
    if !cfg!(target_os = "macos") {
        return Err("application trays are currently supported only on macOS".into());
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !extension.eq_ignore_ascii_case("png") && !extension.eq_ignore_ascii_case("svg") {
        return Err("tray icons must be PNG or SVG files".into());
    }
    load_image(path)?
        .to_image_data(gpui::SvgRenderer::new(std::sync::Arc::new(())))
        .map_err(|error| format!("cannot decode tray icon: {error}"))?;
    Ok(())
}

pub(super) fn load_image(path: &Path) -> Result<gpui::Image, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("cannot read tray icon {}: {error}", path.display()))?;
    let format = if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
    {
        gpui::ImageFormat::Svg
    } else {
        gpui::ImageFormat::Png
    };
    Ok(gpui::Image::from_bytes(format, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_malformed_icons_are_rejected() {
        assert!(validate(None).is_ok());
        assert!(validate(Some(Path::new("missing.png"))).is_err());
        assert!(validate(Some(Path::new("icon.jpg"))).is_err());
        let directory = tempfile::tempdir().unwrap();
        for name in ["invalid.svg", "invalid.png"] {
            let path = directory.path().join(name);
            std::fs::write(&path, b"not an image").unwrap();
            assert!(validate(Some(&path)).is_err());
        }
    }

    #[test]
    fn bundled_icon_is_valid_only_on_supported_platforms() {
        let icon = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/tray/icon.svg");
        assert_eq!(validate(Some(&icon)).is_ok(), cfg!(target_os = "macos"));
    }
}
