use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt as _;
use gpui::AppContext as _;

use crate::lua_runtime::LuaRuntime;
use crate::renderer::{EventCallback, GpuixView};
use crate::retained_tree::RetainedTree;
use crate::text::SharedSelection;

#[derive(Clone, Debug)]
pub struct LuaAppOptions {
    pub title: Option<String>,
    pub width: f32,
    pub height: f32,
    pub watch: bool,
}

impl Default for LuaAppOptions {
    fn default() -> Self {
        Self {
            title: None,
            width: 800.0,
            height: 600.0,
            watch: false,
        }
    }
}

pub fn check_lua_file(path: impl AsRef<Path>) -> Result<(), String> {
    let path = path.as_ref();
    let mut tree = RetainedTree::new();
    LuaRuntime::load_file(path, &mut tree)?;
    Ok(())
}

pub fn run_lua_file(path: impl AsRef<Path>, options: LuaAppOptions) -> Result<(), String> {
    let path = path.as_ref().to_path_buf();
    let tree = Arc::new(Mutex::new(RetainedTree::new()));
    let runtime = {
        let mut tree = tree.lock().unwrap();
        LuaRuntime::load_file(&path, &mut tree)?
    };
    let runtime = Arc::new(Mutex::new(runtime));
    let title = options
        .title
        .clone()
        .unwrap_or_else(|| default_title(&path));
    let app_name = title.clone();
    let startup_error = Arc::new(Mutex::new(None));
    let startup_error_for_app = startup_error.clone();

    gpui_platform::application()
        .with_quit_mode(gpui::QuitMode::LastWindowClosed)
        .run(move |cx| {
            crate::renderer::init_lua_focus_key_bindings(cx);
            crate::custom_elements::input::init(cx);
            #[cfg(target_os = "macos")]
            crate::app_menu::init(&app_name, cx);

            let bounds = gpui::Bounds::centered(
                None,
                gpui::size(gpui::px(options.width), gpui::px(options.height)),
                cx,
            );
            let (event_sender, mut event_receiver) = futures::channel::mpsc::unbounded();
            let callback: EventCallback = Arc::new(move |payload| {
                if event_sender.unbounded_send(payload).is_err() {
                    log::error!("The native Lua event loop is no longer running");
                }
            });
            let tree_for_view = tree.clone();
            let selection = SharedSelection::default();
            let window = match cx.open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_window, cx| {
                    cx.new(|_| {
                        GpuixView::new(tree_for_view, Some(callback), title, selection)
                            .with_native_tab_navigation()
                    })
                },
            ) {
                Ok(window) => window,
                Err(error) => {
                    *startup_error_for_app.lock().unwrap() = Some(error.to_string());
                    cx.quit();
                    return;
                }
            };

            let tree_for_events = tree.clone();
            let runtime_for_events = runtime.clone();
            let window_for_events = window.clone();
            cx.spawn(async move |cx| {
                while let Some(payload) = event_receiver.next().await {
                    let (changed, focus_request) = {
                        let mut runtime = runtime_for_events.lock().unwrap();
                        let mut tree = tree_for_events.lock().unwrap();
                        match runtime.dispatch_event(payload, &mut tree) {
                            Ok(changed) => (changed, runtime.take_focus_request()),
                            Err(error) => {
                                log::error!("Lua event failed: {error}");
                                (false, None)
                            }
                        }
                    };
                    if changed || focus_request.is_some() {
                        window_for_events
                            .update(cx, |view, window, cx| {
                                if let Some(id) = focus_request {
                                    view.reveal_virtual_list_ancestor(id);
                                    if let Some(handle) = view.focus_handles.get(&id) {
                                        handle.focus(window, cx);
                                    }
                                }
                                cx.notify();
                            })
                            .ok();
                    }
                }
            })
            .detach();

            if options.watch {
                let watch_root = path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf();
                let watch_path = path.clone();
                let runtime_for_reload = runtime.clone();
                let tree_for_reload = tree.clone();
                let mut reload_receiver = watch_sources(watch_root);
                cx.spawn(async move |cx| {
                    while reload_receiver.next().await.is_some() {
                        let result = {
                            let mut runtime = runtime_for_reload.lock().unwrap();
                            let mut tree = tree_for_reload.lock().unwrap();
                            runtime.reload_file(&watch_path, &mut tree)
                        };
                        match result {
                            Ok(crate::lua_runtime::ReloadOutcome::PreservedState) => {
                                eprintln!("gpuix-lua: reloaded (state preserved)");
                                window.update(cx, |_view, _window, cx| cx.notify()).ok();
                            }
                            Ok(crate::lua_runtime::ReloadOutcome::ResetState) => {
                                eprintln!(
                                    "gpuix-lua: reloaded (hook signature changed; affected state reset)"
                                );
                                window.update(cx, |_view, _window, cx| cx.notify()).ok();
                            }
                            Err(error) => eprintln!("gpuix-lua: reload failed: {error}"),
                        }
                    }
                })
                .detach();
            }
            cx.activate(true);
        });

    let startup_error = startup_error.lock().unwrap().take();
    match startup_error {
        Some(error) => Err(format!("Failed to open the GPUI window: {error}")),
        None => Ok(()),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SourceSnapshot(Vec<(std::path::PathBuf, u64)>);

fn source_snapshot(root: &Path) -> std::io::Result<SourceSnapshot> {
    let mut sources = Vec::new();
    collect_sources(root, &mut sources)?;
    sources.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    Ok(SourceSnapshot(sources))
}

fn watch_sources(root: std::path::PathBuf) -> futures::channel::mpsc::UnboundedReceiver<()> {
    let (sender, receiver) = futures::channel::mpsc::unbounded();
    eprintln!("gpuix-lua: watching {}", root.display());
    std::thread::spawn(move || {
        let mut observed = match source_snapshot(&root) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                eprintln!("gpuix-lua: cannot watch {}: {error}", root.display());
                return;
            }
        };
        let mut pending_reload = false;
        loop {
            std::thread::sleep(Duration::from_millis(100));
            if sender.is_closed() {
                return;
            }
            let current = match source_snapshot(&root) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    eprintln!(
                        "gpuix-lua: cannot scan {} for changes: {error}",
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
    sources: &mut Vec<(std::path::PathBuf, u64)>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_sources(&path, sources)?;
            continue;
        }
        if !file_type.is_file() || !is_lua_source(&path) {
            continue;
        }
        let bytes = std::fs::read(&path)?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        sources.push((path, hasher.finish()));
    }
    Ok(())
}

fn is_lua_source(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("lua") || extension.eq_ignore_ascii_case("luax")
        })
}

fn default_title(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("GPUIX Lua")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element_tree::EventPayload;

    #[test]
    fn title_defaults_to_the_source_stem() {
        assert_eq!(default_title(Path::new("examples/counter.luax")), "counter");
        assert_eq!(default_title(Path::new("")), "GPUIX Lua");
    }

    #[test]
    fn representative_workspace_exercises_imports_components_and_state() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/luax-workspace/main.luax");
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_file(&path, &mut tree).unwrap();

        assert!(
            tree.elements.len() > 80,
            "workspace retained {} elements",
            tree.elements.len()
        );
        assert!(tree
            .elements
            .values()
            .any(|element| element.element_type == "markdown"));
        assert!(tree
            .elements
            .values()
            .any(|element| element.element_type == "virtual-list"));
        let conversation_list = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("conversation-list"))
            .unwrap();
        assert_eq!(
            conversation_list.custom_props.get("tabIndex"),
            Some(&serde_json::json!(0))
        );
        assert!(!tree.elements.values().any(|element| {
            element
                .test_id
                .as_deref()
                .is_some_and(|test_id| test_id.starts_with("conversation-"))
                && element.test_id.as_deref() != Some("conversation-list")
                && element.custom_props.contains_key("tabIndex")
        }));
        assert!(tree
            .elements
            .values()
            .any(|element| element.test_id.as_deref() == Some("luax-workspace-panel-activity")));
        assert!(tree.elements.values().any(|element| {
            element.test_id.as_deref() == Some("activity-dock-icon")
                && element.element_type == "svg"
        }));
        dispatch_test_id(
            &mut runtime,
            &mut tree,
            "luax-workspace-button-activity",
            "click",
            None,
        );
        assert!(!tree
            .elements
            .values()
            .any(|element| element.test_id.as_deref() == Some("luax-workspace-panel-activity")));
        dispatch_test_id(
            &mut runtime,
            &mut tree,
            "luax-workspace-button-activity",
            "click",
            None,
        );
        assert!(tree
            .elements
            .values()
            .any(|element| element.test_id.as_deref() == Some("luax-workspace-panel-activity")));

        dispatch_key_test_id(&mut runtime, &mut tree, "conversation-list", "down");
        assert_eq!(content_count(&tree, "Binary protocol"), 2);
        assert_eq!(content_count(&tree, "Embedded Lua runtime"), 1);
        dispatch_key_test_id(&mut runtime, &mut tree, "conversation-list", "down");
        assert_eq!(content_count(&tree, "Native text selection"), 2);
        assert_eq!(content_count(&tree, "Binary protocol"), 1);
        dispatch_key_test_id(&mut runtime, &mut tree, "conversation-list", "up");
        assert_eq!(content_count(&tree, "Binary protocol"), 2);
        assert_eq!(content_count(&tree, "Native text selection"), 1);

        dispatch_test_id(&mut runtime, &mut tree, "show-diff", "click", None);
        assert!(tree.elements.values().any(|element| {
            element.test_id.as_deref() == Some("workspace-diff") && element.element_type == "diff"
        }));

        dispatch_test_id(&mut runtime, &mut tree, "show-widgets", "click", None);
        assert!(tree.elements.values().any(|element| {
            element.test_id.as_deref() == Some("showcase-image") && element.element_type == "img"
        }));
        dispatch_test_id(
            &mut runtime,
            &mut tree,
            "notifications-checkbox-indicator",
            "click",
            None,
        );
        dispatch_test_id(
            &mut runtime,
            &mut tree,
            "density-compact-indicator",
            "click",
            None,
        );
        assert!(tree.elements.values().any(|element| {
            element.content.as_deref() == Some("State: notifications off · compact · lua54 · Taffy")
        }));

        dispatch_test_id(&mut runtime, &mut tree, "runtime-select", "click", None);
        assert!(tree
            .elements
            .values()
            .any(|element| { element.test_id.as_deref() == Some("runtime-select-content") }));
        dispatch_test_id(
            &mut runtime,
            &mut tree,
            "runtime-select-option-luajit",
            "click",
            None,
        );
        assert!(tree.elements.values().any(|element| {
            element.content.as_deref()
                == Some("State: notifications off · compact · luajit · Taffy")
        }));
        assert!(!tree
            .elements
            .values()
            .any(|element| { element.test_id.as_deref() == Some("runtime-select-content") }));

        dispatch_test_id(
            &mut runtime,
            &mut tree,
            "framework-combobox",
            "change",
            Some("ru"),
        );
        assert!(tree.elements.values().any(|element| {
            element.test_id.as_deref() == Some("framework-combobox-option-Rust")
        }));
        dispatch_test_id(
            &mut runtime,
            &mut tree,
            "framework-combobox-option-Rust",
            "click",
            None,
        );
        assert!(tree.elements.values().any(|element| {
            element.content.as_deref() == Some("State: notifications off · compact · luajit · Rust")
        }));

        dispatch_test_id(
            &mut runtime,
            &mut tree,
            "controls-tooltip",
            "mouseEnter",
            None,
        );
        assert!(tree
            .elements
            .values()
            .any(|element| { element.test_id.as_deref() == Some("controls-tooltip-content") }));
        dispatch_test_id(
            &mut runtime,
            &mut tree,
            "controls-tooltip",
            "mouseLeave",
            None,
        );
        assert!(!tree
            .elements
            .values()
            .any(|element| { element.test_id.as_deref() == Some("controls-tooltip-content") }));

        dispatch_test_id(&mut runtime, &mut tree, "show-popover", "click", None);
        assert!(tree.elements.values().any(|element| {
            element.test_id.as_deref() == Some("widget-popover")
                && element.element_type == "anchored"
        }));
        dispatch_test_id(
            &mut runtime,
            &mut tree,
            "widget-popover-close",
            "click",
            None,
        );
        assert!(!tree
            .elements
            .values()
            .any(|element| element.test_id.as_deref() == Some("widget-popover")));

        dispatch_test_id(&mut runtime, &mut tree, "show-code", "click", None);
        assert!(tree
            .elements
            .values()
            .any(|element| element.element_type == "code"));

        dispatch_test_id(&mut runtime, &mut tree, "toggle-sidebar", "click", None);
        assert!(tree
            .elements
            .values()
            .any(|element| element.content.as_deref() == Some("Expand")));

        dispatch_test_id(&mut runtime, &mut tree, "activity-7", "click", None);
        dispatch_test_id(
            &mut runtime,
            &mut tree,
            "workspace-composer",
            "change",
            Some("native input"),
        );
        assert!(tree.elements.values().any(|element| {
            element.test_id.as_deref() == Some("workspace-composer")
                && element.custom_props.get("value")
                    == Some(&serde_json::Value::String("native input".to_string()))
        }));

        dispatch_test_id(&mut runtime, &mut tree, "send-message", "click", None);
        assert!(tree
            .elements
            .values()
            .any(|element| element.content.as_deref() == Some("Send (1)")));
        assert!(tree.elements.values().any(|element| {
            element.test_id.as_deref() == Some("workspace-composer")
                && element.custom_props.get("value")
                    == Some(&serde_json::Value::String(String::new()))
        }));
    }

    #[test]
    fn source_watcher_reports_a_stable_lua_change() {
        let root = std::env::temp_dir().join(format!(
            "gpuix-watch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("main.lua");
        std::fs::write(&source, "return function() end").unwrap();
        let mut receiver = watch_sources(root.clone());
        std::thread::sleep(Duration::from_millis(150));
        std::fs::write(&source, "return function() return nil end").unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let received = loop {
            match receiver.try_recv() {
                Ok(()) => break true,
                Err(futures::channel::mpsc::TryRecvError::Closed) => break false,
                Err(_) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(_) => break false,
            }
        };
        std::fs::remove_dir_all(root).ok();
        assert!(received);
    }

    fn dispatch_test_id(
        runtime: &mut LuaRuntime,
        tree: &mut RetainedTree,
        test_id: &str,
        event_type: &str,
        value: Option<&str>,
    ) {
        let element_id = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some(test_id))
            .unwrap()
            .id;
        assert!(runtime
            .dispatch_event(
                EventPayload {
                    element_id: element_id as f64,
                    event_type: event_type.to_string(),
                    value: value.map(str::to_string),
                    ..Default::default()
                },
                tree,
            )
            .unwrap());
    }

    fn dispatch_key_test_id(
        runtime: &mut LuaRuntime,
        tree: &mut RetainedTree,
        test_id: &str,
        key: &str,
    ) {
        let element_id = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some(test_id))
            .unwrap()
            .id;
        assert!(runtime
            .dispatch_event(
                EventPayload {
                    element_id: element_id as f64,
                    event_type: "keyDown".to_string(),
                    key: Some(key.to_string()),
                    ..Default::default()
                },
                tree,
            )
            .unwrap());
    }

    fn content_count(tree: &RetainedTree, content: &str) -> usize {
        tree.elements
            .values()
            .filter(|element| element.content.as_deref() == Some(content))
            .count()
    }
}
