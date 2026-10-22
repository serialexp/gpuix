//! What the embedded script engines share: the application and window
//! registry, the source watcher for hot reload, and the options `gpx` passes
//! on. Each engine keeps its own values (render callbacks, handlers) and its
//! own error wording.

pub(crate) mod app;
pub(crate) mod reload;
pub(crate) mod server;
pub(crate) mod tray;
pub(crate) mod tray_api;

/// How `gpx` runs an app, from its command line.
#[derive(Clone, Debug, Default)]
pub struct AppOptions {
    /// The window title. Beats the app's own.
    pub title: Option<String>,
    /// The window size. Beats the app's own.
    pub width: Option<f32>,
    pub height: Option<f32>,
    /// Reload the app when a source file under its directory changes.
    pub watch: bool,
    pub background: bool,
    pub dev_server: Option<u16>,
}

/// An app's window title when neither the app nor the command line sets one:
/// its entry file's name, without the extension.
pub(crate) fn default_title(entry: &std::path::Path) -> String {
    entry
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("GPUIX")
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn title_defaults_to_the_entry_file_name() {
        assert_eq!(default_title(Path::new("examples/counter.luax")), "counter");
        assert_eq!(default_title(Path::new("app/main.tsx")), "main");
        assert_eq!(default_title(Path::new("")), "GPUIX");
    }
}
