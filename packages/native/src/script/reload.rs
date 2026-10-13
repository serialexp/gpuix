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

/// Watch the files under `root` that `is_source` accepts, and send `()`
/// once they change and then hold still for one poll, so an editor that
/// writes a file in several steps triggers one reload. Polls every 100 ms
/// on a thread of its own, which ends when the receiver is dropped.
/// `log_prefix` names the program in messages.
pub(crate) fn watch_sources(
    root: PathBuf,
    is_source: fn(&Path) -> bool,
    log_prefix: &'static str,
) -> futures::channel::mpsc::UnboundedReceiver<()> {
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
            collect_sources(&path, is_source, sources)?;
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
