//! Hot reload bookkeeping that doesn't depend on the engine: watching an
//! app's sources, and what a reload reports back to the app host. Keeping
//! script state across a reload is each engine's own business.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::app::WindowOptions;

/// How a reloaded window's state came through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReloadOutcome {
    PreservedState,
    ResetState,
}

/// The windows of a reloaded application, and the ids of the windows it no
/// longer defines.
pub(crate) struct ApplicationReload {
    pub windows: Vec<WindowOptions>,
    pub removed: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct SourceSnapshot(Vec<(PathBuf, u64)>);

fn source_snapshot(root: &Path, is_source: fn(&Path) -> bool) -> std::io::Result<SourceSnapshot> {
    let mut sources = Vec::new();
    collect_sources(root, is_source, &mut sources)?;
    sources.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    Ok(SourceSnapshot(sources))
}

/// Watch the files in the entry file's directory tree that `is_source`
/// accepts, outside `node_modules` and dot directories, and send `()`
/// once they change and then hold still for one poll, so an editor that
/// writes a file in several steps triggers one reload. Polls every 100 ms
/// on a thread of its own, which ends when the receiver is dropped.
/// `log_prefix` names the program in messages.
pub(crate) fn watch_sources(
    entry: &Path,
    is_source: fn(&Path) -> bool,
    log_prefix: &'static str,
) -> futures::channel::mpsc::UnboundedReceiver<()> {
    let root = entry_directory(entry);
    let (sender, receiver) = futures::channel::mpsc::unbounded();
    eprintln!("{log_prefix}: watching {}", root.display());
    std::thread::spawn(move || {
        let mut observed = match source_snapshot(&root, is_source) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                eprintln!("{log_prefix}: cannot watch {}: {error}", root.display());
                return;
            }
        };
        let mut pending_reload = false;
        loop {
            std::thread::sleep(Duration::from_millis(100));
            if sender.is_closed() {
                return;
            }
            let current = match source_snapshot(&root, is_source) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    eprintln!(
                        "{log_prefix}: cannot scan {} for changes: {error}",
                        root.display()
                    );
                    continue;
                }
            };
            if current != observed {
                observed = current;
                pending_reload = true;
                continue;
            }
            if pending_reload {
                pending_reload = false;
                if sender.unbounded_send(()).is_err() {
                    return;
                }
            }
        }
    });
    receiver
}

/// The directory an entry file is in. A bare file name's parent is the empty
/// path, which `read_dir` rejects, so that becomes the current directory.
fn entry_directory(entry: &Path) -> PathBuf {
    match entry.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Dependencies and dot directories (`.git`, editor state) aren't the app's
/// own sources, and `node_modules` alone can hold many thousands of files.
fn is_skipped_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "node_modules" || name.starts_with('.'))
}

fn collect_sources(
    root: &Path,
    is_source: fn(&Path) -> bool,
    sources: &mut Vec<(PathBuf, u64)>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            if !is_skipped_directory(&path) {
                collect_sources(&path, is_source, sources)?;
            }
            continue;
        }
        if !file_type.is_file() || !is_source(&path) {
            continue;
        }
        let bytes = std::fs::read(&path)?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        sources.push((path, hasher.finish()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_is_watched_from_its_directory() {
        assert_eq!(entry_directory(Path::new("app/main.tsx")), Path::new("app"));
        assert_eq!(entry_directory(Path::new("main.tsx")), Path::new("."));
        assert_eq!(entry_directory(Path::new("/main.tsx")), Path::new("/"));
    }

    #[test]
    fn dependencies_and_dot_directories_are_not_watched() {
        let root = std::env::temp_dir().join(format!("gpuix-snapshot-{}", std::process::id()));
        for directory in ["src", "node_modules/react", ".git"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        for file in [
            "main.ts",
            "src/view.ts",
            "node_modules/react/index.ts",
            ".git/hook.ts",
        ] {
            std::fs::write(root.join(file), file).unwrap();
        }
        let is_ts = |path: &Path| path.extension().is_some_and(|extension| extension == "ts");
        let snapshot = source_snapshot(&root, is_ts);
        std::fs::remove_dir_all(&root).ok();
        let watched: Vec<PathBuf> = snapshot
            .unwrap()
            .0
            .into_iter()
            .map(|(path, _)| path)
            .collect();
        assert_eq!(watched, [root.join("main.ts"), root.join("src/view.ts")]);
    }
}
