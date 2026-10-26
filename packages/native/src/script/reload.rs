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

pub(crate) struct FileWatch {
    files: std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<PathBuf, u64>>>,
    restart: std::sync::Mutex<std::collections::BTreeSet<PathBuf>>,
}

impl FileWatch {
    pub(crate) fn track_restart(&self, paths: Vec<PathBuf>) {
        self.restart.lock().unwrap().extend(paths.iter().cloned());
        self.track(paths);
    }

    pub(crate) fn requires_restart(&self, changed: &[PathBuf]) -> bool {
        let restart = self.restart.lock().unwrap();
        changed.iter().any(|path| restart.contains(path))
    }

    pub(crate) fn track(&self, paths: impl IntoIterator<Item = PathBuf>) {
        let mut files = self.files.lock().unwrap();
        for path in paths {
            if files.contains_key(&path) {
                continue;
            }
            match file_fingerprint(&path) {
                Ok(hash) => {
                    files.insert(path, hash);
                }
                Err(error) => eprintln!("gpx: cannot watch {}: {error}", path.display()),
            }
        }
    }
}

fn file_fingerprint(path: &Path) -> std::io::Result<u64> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::NotADirectory
                    | std::io::ErrorKind::IsADirectory
            ) =>
        {
            None
        }
        Err(error) => return Err(error),
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(hasher.finish())
}

pub(crate) fn watch_files(
    paths: impl IntoIterator<Item = PathBuf>,
) -> (
    FileWatch,
    futures::channel::mpsc::UnboundedReceiver<Vec<PathBuf>>,
) {
    let watcher = FileWatch {
        files: Default::default(),
        restart: Default::default(),
    };
    watcher.track(paths);
    let files = watcher.files.clone();
    let (sender, receiver) = futures::channel::mpsc::unbounded();
    std::thread::spawn(move || {
        let mut pending = std::collections::BTreeSet::new();
        while !sender.is_closed() {
            std::thread::sleep(Duration::from_millis(100));
            let mut changed = false;
            {
                let mut files = files.lock().unwrap();
                for (path, observed) in files.iter_mut() {
                    match file_fingerprint(path) {
                        Ok(hash) if hash != *observed => {
                            *observed = hash;
                            pending.insert(path.clone());
                            changed = true;
                        }
                        Ok(_) => {}
                        Err(error) => eprintln!("gpx: cannot scan {}: {error}", path.display()),
                    }
                }
            }
            if !changed && !pending.is_empty() {
                if sender
                    .unbounded_send(std::mem::take(&mut pending).into_iter().collect())
                    .is_err()
                {
                    return;
                }
            }
        }
    });
    (watcher, receiver)
}

fn source_snapshot(root: &Path, is_source: fn(&Path) -> bool) -> std::io::Result<SourceSnapshot> {
    let mut sources = Vec::new();
    collect_sources(root, is_source, &mut sources)?;
    sources.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    Ok(SourceSnapshot(sources))
}

/// The files added, removed or changed from `before` to `after`.
fn changed_paths(before: &SourceSnapshot, after: &SourceSnapshot) -> Vec<PathBuf> {
    let before: std::collections::HashMap<&PathBuf, u64> =
        before.0.iter().map(|(path, hash)| (path, *hash)).collect();
    let after_paths: std::collections::HashSet<&PathBuf> =
        after.0.iter().map(|(path, _)| path).collect();
    let mut changed: Vec<PathBuf> = after
        .0
        .iter()
        .filter(|(path, hash)| before.get(path) != Some(hash))
        .map(|(path, _)| path.clone())
        .chain(
            before
                .keys()
                .filter(|path| !after_paths.contains(*path))
                .map(|path| (*path).clone()),
        )
        .collect();
    changed.sort_unstable();
    changed
}

/// Watch the files in the entry file's directory tree that `is_source`
/// accepts, outside `node_modules` and dot directories, and send the ones
/// added, removed or changed once they change and then hold still for one
/// poll, so an editor that writes a file in several steps triggers one
/// reload. Polls every 100 ms on a thread of its own, which ends when the
/// receiver is dropped. `log_prefix` names the program in messages.
pub(crate) fn watch_sources(
    entry: &Path,
    is_source: fn(&Path) -> bool,
    log_prefix: &'static str,
) -> futures::channel::mpsc::UnboundedReceiver<Vec<PathBuf>> {
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
        let mut pending = std::collections::BTreeSet::new();
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
                pending.extend(changed_paths(&observed, &current));
                observed = current;
                continue;
            }
            if !pending.is_empty() {
                let changed = std::mem::take(&mut pending).into_iter().collect();
                if sender.unbounded_send(changed).is_err() {
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
    fn explicit_watches_follow_added_paths_deletion_and_recreation() {
        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("shared.txt");
        let config = directory.path().join("tsconfig.json");
        std::fs::write(&outside, "before").unwrap();
        let (watcher, mut receiver) = watch_files([outside.clone()]);
        let mut next = || {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            loop {
                if let Ok(paths) = receiver.try_recv() {
                    break paths;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "watch notification timed out"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        std::fs::write(&outside, "after").unwrap();
        assert_eq!(next(), [outside.clone()]);
        watcher.track([outside.clone(), config.clone()]);
        std::fs::write(&config, "{}").unwrap();
        assert_eq!(next(), [config]);
        std::fs::remove_file(&outside).unwrap();
        assert_eq!(next(), [outside.clone()]);
        std::fs::write(&outside, "restored").unwrap();
        assert_eq!(next(), [outside]);
    }

    #[test]
    fn a_snapshot_change_names_the_files_added_removed_and_changed() {
        let snapshot = |files: &[(&str, u64)]| {
            SourceSnapshot(
                files
                    .iter()
                    .map(|(path, hash)| (PathBuf::from(path), *hash))
                    .collect(),
            )
        };
        let before = snapshot(&[("a.ts", 1), ("b.ts", 2), ("c.ts", 3)]);
        let after = snapshot(&[("a.ts", 1), ("b.ts", 20), ("d.ts", 4)]);
        assert_eq!(
            changed_paths(&before, &after),
            [
                PathBuf::from("b.ts"),
                PathBuf::from("c.ts"),
                PathBuf::from("d.ts")
            ]
        );
    }

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
