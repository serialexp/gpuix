use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt as _;
use gpui::AppContext as _;

use crate::element_tree::EventPayload;
use crate::lua_runtime::{LuaAppCommand, LuaRuntime, LuaWindowOptions};
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
    let mut runtime = LuaRuntime::load_application_file(path)?;
    for window in runtime.window_options() {
        let mut tree = RetainedTree::new();
        runtime.mount_window(&window.id, &mut tree)?;
    }
    Ok(())
}

pub fn run_lua_file(path: impl AsRef<Path>, options: LuaAppOptions) -> Result<(), String> {
    let path = path.as_ref().to_path_buf();
    let runtime = LuaRuntime::load_application_file(&path)?;
    let application_entry = runtime.is_application_entry();
    let mut window_options = runtime.window_options();
    if !application_entry {
        let main = window_options
            .iter_mut()
            .find(|window| window.id == "main")
            .expect("legacy Lua runtime has a main window");
        main.title = options
            .title
            .clone()
            .unwrap_or_else(|| default_title(&path));
        main.width = options.width;
        main.height = options.height;
    }
    if !window_options.iter().any(|window| window.open) {
        return Err("Lua application must open at least one window at startup".to_string());
    }
    let app_name = window_options
        .iter()
        .find(|window| window.open)
        .map(|window| window.title.clone())
        .unwrap();
    let runtime = Arc::new(Mutex::new(runtime));
    let startup_error = Arc::new(Mutex::new(None));
    let startup_error_for_app = startup_error.clone();

    gpui_platform::application()
        .with_quit_mode(gpui::QuitMode::LastWindowClosed)
        .run(move |cx| {
            crate::renderer::init_lua_focus_key_bindings(cx);
            crate::custom_elements::input::init(cx);
            #[cfg(target_os = "macos")]
            crate::app_menu::init(&app_name, cx);

            let (event_sender, mut event_receiver) = futures::channel::mpsc::unbounded();
            let windows = Rc::new(LuaWindows::default());
            let trees = Rc::new(RefCell::new(HashMap::new()));
            let native_ids = Rc::new(RefCell::new(HashMap::new()));
            let window_bounds = Rc::new(LuaWindowBounds::default());
            let initial_windows = window_options.clone();
            let definitions = Rc::new(RefCell::new(
                window_options
                    .into_iter()
                    .map(|window| (window.id.clone(), window))
                    .collect::<HashMap<_, _>>(),
            ));

            for window in initial_windows.iter().filter(|window| window.open) {
                if let Err(error) = open_lua_window(
                    window,
                    &runtime,
                    &event_sender,
                    &windows,
                    &trees,
                    &native_ids,
                    &window_bounds,
                    cx,
                ) {
                    *startup_error_for_app.lock().unwrap() = Some(error);
                    cx.quit();
                    return;
                }
            }

            let close_windows = windows.clone();
            let close_trees = trees.clone();
            let close_native_ids = native_ids.clone();
            let close_runtime = runtime.clone();
            cx.on_window_closed(move |cx, native_id| {
                crate::text::paint::remove_window(native_id);
                let id = close_native_ids.borrow_mut().remove(&native_id);
                let Some(id) = id else {
                    return;
                };
                {
                    close_windows.remove(&id);
                }
                {
                    close_trees.borrow_mut().remove(&id);
                }
                close_runtime.lock().unwrap().unmount_window(&id);
                reconcile_dirty_windows(
                    &close_runtime,
                    &close_windows,
                    &close_trees,
                    cx,
                );
            })
            .detach();

            let runtime_for_events = runtime.clone();
            let windows_for_events = windows.clone();
            let trees_for_events = trees.clone();
            let native_ids_for_events = native_ids.clone();
            let window_bounds_for_events = window_bounds.clone();
            let definitions_for_events = definitions.clone();
            let event_sender_for_events = event_sender.clone();
            cx.spawn(async move |cx| {
                while let Some((source_id, payload)) = event_receiver.next().await {
                    let (changed, focus_request, commands) = {
                        let mut runtime = runtime_for_events.lock().unwrap();
                        match runtime.dispatch_window_event(&source_id, payload) {
                            Ok(dirty_roots) => {
                                let mut changed = Vec::new();
                                for id in dirty_roots {
                                    let tree = trees_for_events.borrow().get(&id).cloned();
                                    let Some(tree) = tree else {
                                        continue;
                                    };
                                    let result = {
                                        let mut tree = tree.lock().unwrap();
                                        runtime.mount_window(&id, &mut tree)
                                    };
                                    match result {
                                        Ok(()) => changed.push(id),
                                        Err(error) => {
                                            log::error!("Lua window {id:?} render failed: {error}")
                                        }
                                    }
                                }
                                (
                                    changed,
                                    runtime.take_focus_request(),
                                    runtime.take_app_commands(),
                                )
                            }
                            Err(error) => {
                                log::error!("Lua event failed: {error}");
                                (Vec::new(), None, runtime.take_app_commands())
                            }
                        }
                    };

                    for id in changed {
                        if let Some(window) = window_handle(&windows_for_events, &id) {
                            window.update(cx, |_view, _window, cx| cx.notify()).ok();
                        }
                    }
                    if let Some(request) = focus_request {
                        let window = window_handle(&windows_for_events, &request.root_id);
                        if let Some(window) = window {
                            window
                                .update(cx, |view, window, cx| {
                                    view.reveal_virtual_list_ancestor(request.element_id);
                                    if let Some(handle) =
                                        view.focus_handles.get(&request.element_id)
                                    {
                                        handle.focus(window, cx);
                                    }
                                })
                                .ok();
                        }
                    }

                    for command in commands {
                        apply_app_command(
                            command,
                            &runtime_for_events,
                            &event_sender_for_events,
                            &windows_for_events,
                            &trees_for_events,
                            &native_ids_for_events,
                            &window_bounds_for_events,
                            &definitions_for_events,
                            cx,
                        );
                    }
                    reconcile_dirty_windows(
                        &runtime_for_events,
                        &windows_for_events,
                        &trees_for_events,
                        cx,
                    );
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
                let windows_for_reload = windows.clone();
                let trees_for_reload = trees.clone();
                let native_ids_for_reload = native_ids.clone();
                let window_bounds_for_reload = window_bounds.clone();
                let definitions_for_reload = definitions.clone();
                let event_sender_for_reload = event_sender.clone();
                let mut reload_receiver = watch_sources(watch_root);
                cx.spawn(async move |cx| {
                    while reload_receiver.next().await.is_some() {
                        if application_entry {
                            let reload = runtime_for_reload
                                .lock()
                                .unwrap()
                                .reload_application_file(&watch_path);
                            let reload = match reload {
                                Ok(reload) => reload,
                                Err(error) => {
                                    eprintln!("gpuix-lua: reload failed: {error}");
                                    continue;
                                }
                            };
                            *definitions_for_reload.borrow_mut() = reload
                                .windows
                                .iter()
                                .cloned()
                                .map(|window| (window.id.clone(), window))
                                .collect();
                            for id in reload.removed {
                                if let Some(window) = window_handle(&windows_for_reload, &id) {
                                    window
                                        .update(cx, |_view, window, _cx| window.remove_window())
                                        .ok();
                                }
                            }

                            let open_ids = windows_for_reload.ids();
                            let mut reset_state = false;
                            for id in open_ids {
                                let Some(options) =
                                    definitions_for_reload.borrow().get(&id).cloned()
                                else {
                                    continue;
                                };
                                let Some(tree) = trees_for_reload.borrow().get(&id).cloned() else {
                                    continue;
                                };
                                let result = {
                                    let mut runtime = runtime_for_reload.lock().unwrap();
                                    let mut tree = tree.lock().unwrap();
                                    runtime.refresh_window(&id, &mut tree)
                                };
                                match result {
                                    Ok(crate::lua_runtime::ReloadOutcome::PreservedState) => {}
                                    Ok(crate::lua_runtime::ReloadOutcome::ResetState) => {
                                        reset_state = true
                                    }
                                    Err(error) => {
                                        eprintln!("gpuix-lua: window {id:?} reload failed: {error}");
                                        continue;
                                    }
                                }
                                if let Some(window) = window_handle(&windows_for_reload, &id) {
                                    window
                                        .update(cx, |view, _window, cx| {
                                            view.window_title = options.title;
                                            cx.notify();
                                        })
                                        .ok();
                                }
                            }
                            let new_windows = reload
                                .windows
                                .iter()
                                .filter(|window| {
                                    window.open && !windows_for_reload.contains(&window.id)
                                })
                                .cloned()
                                .collect::<Vec<_>>();
                            for window in new_windows {
                                let result = cx.update(|cx| {
                                    open_lua_window(
                                        &window,
                                        &runtime_for_reload,
                                        &event_sender_for_reload,
                                        &windows_for_reload,
                                        &trees_for_reload,
                                        &native_ids_for_reload,
                                        &window_bounds_for_reload,
                                        cx,
                                    )
                                });
                                if let Err(error) = result {
                                    eprintln!("gpuix-lua: reload failed: {error}");
                                }
                            }
                            if reset_state {
                                eprintln!(
                                    "gpuix-lua: reloaded (hook signature changed; affected state reset)"
                                );
                            } else {
                                eprintln!("gpuix-lua: reloaded (state preserved)");
                            }
                            continue;
                        }
                        let Some(tree) = trees_for_reload.borrow().get("main").cloned() else {
                            continue;
                        };
                        let result = runtime_for_reload
                            .lock()
                            .unwrap()
                            .reload_file(&watch_path, &mut tree.lock().unwrap());
                        match result {
                            Ok(crate::lua_runtime::ReloadOutcome::PreservedState) => {
                                eprintln!("gpuix-lua: reloaded (state preserved)")
                            }
                            Ok(crate::lua_runtime::ReloadOutcome::ResetState) => eprintln!(
                                "gpuix-lua: reloaded (hook signature changed; affected state reset)"
                            ),
                            Err(error) => {
                                eprintln!("gpuix-lua: reload failed: {error}");
                                continue;
                            }
                        }
                        if let Some(window) = window_handle(&windows_for_reload, "main") {
                            window.update(cx, |_view, _window, cx| cx.notify()).ok();
                        }
                    }
                })
                .detach();
            }
            if initial_windows.iter().any(|window| window.open && window.focus) {
                cx.activate(true);
            }
        });

    let startup_error = startup_error.lock().unwrap().take();
    match startup_error {
        Some(error) => Err(format!("Failed to open the GPUI window: {error}")),
        None => Ok(()),
    }
}

type LuaEventSender = futures::channel::mpsc::UnboundedSender<(String, EventPayload)>;
#[derive(Default)]
struct LuaWindows(RefCell<HashMap<String, gpui::WindowHandle<GpuixView>>>);

impl LuaWindows {
    fn contains(&self, id: &str) -> bool {
        self.0.borrow().contains_key(id)
    }

    fn get(&self, id: &str) -> Option<gpui::WindowHandle<GpuixView>> {
        self.0.borrow().get(id).copied()
    }

    fn ids(&self) -> Vec<String> {
        self.0.borrow().keys().cloned().collect()
    }

    fn insert(&self, id: String, window: gpui::WindowHandle<GpuixView>) {
        self.0.borrow_mut().insert(id, window);
    }

    fn remove(&self, id: &str) {
        self.0.borrow_mut().remove(id);
    }
}

type SharedLuaWindows = Rc<LuaWindows>;

#[derive(Default)]
struct LuaWindowBounds(RefCell<HashMap<String, gpui::WindowBounds>>);

impl LuaWindowBounds {
    fn get(&self, id: &str) -> Option<gpui::WindowBounds> {
        self.0.borrow().get(id).copied()
    }

    fn insert(&self, id: String, bounds: gpui::WindowBounds) {
        self.0.borrow_mut().insert(id, bounds);
    }
}

type SharedLuaWindowBounds = Rc<LuaWindowBounds>;
type LuaTrees = Rc<RefCell<HashMap<String, Arc<Mutex<RetainedTree>>>>>;
type LuaNativeWindowIds = Rc<RefCell<HashMap<gpui::WindowId, String>>>;
type LuaWindowDefinitions = Rc<RefCell<HashMap<String, LuaWindowOptions>>>;

fn window_handle(windows: &SharedLuaWindows, id: &str) -> Option<gpui::WindowHandle<GpuixView>> {
    windows.get(id)
}

fn open_lua_window(
    options: &LuaWindowOptions,
    runtime: &Arc<Mutex<LuaRuntime>>,
    event_sender: &LuaEventSender,
    windows: &SharedLuaWindows,
    trees: &LuaTrees,
    native_ids: &LuaNativeWindowIds,
    saved_bounds: &SharedLuaWindowBounds,
    cx: &mut gpui::App,
) -> Result<(), String> {
    if windows.contains(&options.id) {
        return Ok(());
    }
    let tree = Arc::new(Mutex::new(RetainedTree::new()));
    runtime
        .lock()
        .unwrap()
        .mount_window(&options.id, &mut tree.lock().unwrap())?;

    let id = options.id.clone();
    let sender = event_sender.clone();
    let callback: EventCallback = Arc::new(move |payload| {
        if sender.unbounded_send((id.clone(), payload)).is_err() {
            log::error!("The native Lua event loop is no longer running");
        }
    });
    let bounds = remembered_window_bounds(&options.id, options.reposition, saved_bounds)
        .unwrap_or_else(|| {
            gpui::WindowBounds::centered(
                gpui::size(gpui::px(options.width), gpui::px(options.height)),
                cx,
            )
        });
    let tree_for_view = tree.clone();
    let title = options.title.clone();
    let bounds_for_view = saved_bounds.clone();
    let id_for_view = options.id.clone();
    let window = match cx.open_window(
        gpui::WindowOptions {
            window_bounds: Some(bounds),
            focus: options.focus,
            ..Default::default()
        },
        |window, cx| {
            cx.new(|cx| {
                bounds_for_view.insert(id_for_view.clone(), window.window_bounds());
                let observed_bounds = bounds_for_view.clone();
                let observed_id = id_for_view.clone();
                cx.observe_window_bounds(window, move |_, window, _| {
                    observed_bounds.insert(observed_id.clone(), window.window_bounds());
                })
                .detach();
                GpuixView::new(
                    tree_for_view,
                    Some(callback),
                    title,
                    SharedSelection::default(),
                )
                .with_native_tab_navigation()
            })
        },
    ) {
        Ok(window) => window,
        Err(error) => {
            runtime.lock().unwrap().unmount_window(&options.id);
            return Err(format!(
                "Failed to open Lua window {:?}: {error}",
                options.id
            ));
        }
    };
    native_ids
        .borrow_mut()
        .insert(window.window_id(), options.id.clone());
    trees.borrow_mut().insert(options.id.clone(), tree);
    windows.insert(options.id.clone(), window);
    runtime.lock().unwrap().set_window_open(&options.id, true);
    Ok(())
}

fn apply_app_command(
    command: LuaAppCommand,
    runtime: &Arc<Mutex<LuaRuntime>>,
    event_sender: &LuaEventSender,
    windows: &SharedLuaWindows,
    trees: &LuaTrees,
    native_ids: &LuaNativeWindowIds,
    saved_bounds: &SharedLuaWindowBounds,
    definitions: &LuaWindowDefinitions,
    cx: &mut gpui::AsyncApp,
) {
    match command {
        LuaAppCommand::Open(id) => {
            if let Some(window) = window_handle(windows, &id) {
                window
                    .update(cx, |_view, window, cx| {
                        cx.activate(true);
                        window.activate_window();
                    })
                    .ok();
                return;
            }
            let Some(options) = definitions.borrow().get(&id).cloned() else {
                return;
            };
            let result = cx.update(|cx| {
                open_lua_window(
                    &options,
                    runtime,
                    event_sender,
                    windows,
                    trees,
                    native_ids,
                    saved_bounds,
                    cx,
                )
            });
            if let Err(error) = result {
                log::error!("{error}");
            }
        }
        LuaAppCommand::Close(id) => {
            if let Some(window) = window_handle(windows, &id) {
                let saved_bounds = saved_bounds.clone();
                window
                    .update(cx, move |_view, window, _cx| {
                        saved_bounds.insert(id, window.window_bounds());
                        window.remove_window();
                    })
                    .ok();
            }
        }
        LuaAppCommand::Focus(id) => {
            if let Some(window) = window_handle(windows, &id) {
                window
                    .update(cx, |_view, window, cx| {
                        cx.activate(true);
                        window.activate_window();
                    })
                    .ok();
            }
        }
        LuaAppCommand::SetTitle { id, title } => {
            if let Some(options) = definitions.borrow_mut().get_mut(&id) {
                options.title = title.clone();
            }
            if let Some(window) = window_handle(windows, &id) {
                window
                    .update(cx, |view, _window, cx| {
                        view.window_title = title;
                        cx.notify();
                    })
                    .ok();
            }
        }
    }
}

fn remembered_window_bounds(
    id: &str,
    reposition: bool,
    saved_bounds: &SharedLuaWindowBounds,
) -> Option<gpui::WindowBounds> {
    (!reposition).then(|| saved_bounds.get(id)).flatten()
}

fn reconcile_dirty_windows<C: gpui::AppContext>(
    runtime: &Arc<Mutex<LuaRuntime>>,
    windows: &SharedLuaWindows,
    trees: &LuaTrees,
    cx: &mut C,
) {
    let dirty = runtime.lock().unwrap().dirty_window_ids();
    let mut changed = Vec::new();
    for id in dirty {
        let Some(tree) = trees.borrow().get(&id).cloned() else {
            continue;
        };
        let result = {
            let mut runtime = runtime.lock().unwrap();
            let mut tree = tree.lock().unwrap();
            runtime.mount_window(&id, &mut tree)
        };
        match result {
            Ok(()) => changed.push(id),
            Err(error) => log::error!("Lua window {id:?} render failed: {error}"),
        }
    }
    for id in changed {
        if let Some(window) = window_handle(windows, &id) {
            window.update(cx, |_view, _window, cx| cx.notify()).ok();
        }
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
    fn remembered_bounds_are_used_unless_repositioning_is_requested() {
        let saved_bounds = Rc::new(LuaWindowBounds::default());
        let bounds = gpui::WindowBounds::Windowed(gpui::Bounds::new(
            gpui::point(gpui::px(120.0), gpui::px(80.0)),
            gpui::size(gpui::px(640.0), gpui::px(480.0)),
        ));
        saved_bounds.insert("inspector".to_string(), bounds);

        assert_eq!(
            remembered_window_bounds("inspector", false, &saved_bounds),
            Some(bounds)
        );
        assert_eq!(
            remembered_window_bounds("inspector", true, &saved_bounds),
            None
        );
    }

    #[test]
    fn representative_workspace_exercises_imports_components_and_state() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/luax-workspace/main.luax");
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_application_file(&path).unwrap();
        runtime.mount_window("main", &mut tree).unwrap();

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
        assert!(!has_foreground_box_shadow(&tree, "conversation-runtime"));
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
        let activity_focus_is_inset = tree.elements.values().any(|element| {
            element.test_id.as_deref() == Some("activity-1")
                && element
                    .style
                    .as_ref()
                    .and_then(|style| style.focus.as_deref())
                    .and_then(|style| style.foreground_box_shadow.as_ref())
                    .is_some_and(|shadow| shadow.inset)
        });
        assert!(activity_focus_is_inset);
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

        dispatch_test_id(&mut runtime, &mut tree, "conversation-list", "focus", None);
        assert!(has_foreground_box_shadow(&tree, "conversation-runtime"));
        dispatch_key_test_id(&mut runtime, &mut tree, "conversation-list", "down");
        assert_eq!(content_count(&tree, "Embedded Lua runtime"), 2);
        assert_eq!(content_count(&tree, "Binary protocol"), 1);
        assert!(has_foreground_box_shadow(&tree, "conversation-protocol"));
        assert!(!has_foreground_box_shadow(&tree, "conversation-runtime"));
        dispatch_key_test_id(&mut runtime, &mut tree, "conversation-list", "enter");
        assert_eq!(content_count(&tree, "Binary protocol"), 2);
        assert_eq!(content_count(&tree, "Embedded Lua runtime"), 1);
        dispatch_key_test_id(&mut runtime, &mut tree, "conversation-list", "down");
        assert_eq!(content_count(&tree, "Binary protocol"), 2);
        assert_eq!(content_count(&tree, "Native text selection"), 1);
        assert!(has_foreground_box_shadow(&tree, "conversation-selection"));
        dispatch_key_test_id(&mut runtime, &mut tree, "conversation-list", "space");
        assert_eq!(content_count(&tree, "Native text selection"), 2);
        assert_eq!(content_count(&tree, "Binary protocol"), 1);
        dispatch_test_id(&mut runtime, &mut tree, "conversation-list", "blur", None);
        assert!(!has_foreground_box_shadow(&tree, "conversation-selection"));

        dispatch_test_id(&mut runtime, &mut tree, "show-diff", "click", None);
        assert!(tree.elements.values().any(|element| {
            element.test_id.as_deref() == Some("workspace-diff") && element.element_type == "diff"
        }));

        dispatch_test_id(&mut runtime, &mut tree, "show-widgets", "click", None);
        assert!(tree.elements.values().any(|element| {
            element.test_id.as_deref() == Some("showcase-image") && element.element_type == "img"
        }));
        let navbar_style = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("workspace-navbar"))
            .and_then(|element| element.style.as_ref())
            .unwrap();
        assert_eq!(navbar_style.flex_shrink, Some(0.0));
        let sidebar_style = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("workspace-sidebar"))
            .and_then(|element| element.style.as_ref())
            .unwrap();
        assert_eq!(sidebar_style.flex_shrink, Some(0.0));
        let content_style = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("workspace-content"))
            .and_then(|element| element.style.as_ref())
            .unwrap();
        assert_eq!(content_style.flex_basis, Some(0.0));
        let body_style = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("workspace-body"))
            .and_then(|element| element.style.as_ref())
            .unwrap();
        assert_eq!(body_style.flex_basis, Some(0.0));
        assert_eq!(
            body_style.min_height,
            Some(crate::style::DimensionValue::Pixels(0.0))
        );
        let detail_style = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("workspace-detail"))
            .and_then(|element| element.style.as_ref())
            .unwrap();
        assert_eq!(
            detail_style.min_height,
            Some(crate::style::DimensionValue::Pixels(0.0))
        );
        let composer_style = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("workspace-composer-row"))
            .and_then(|element| element.style.as_ref())
            .unwrap();
        assert_eq!(composer_style.flex_shrink, Some(0.0));
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
        dispatch_key_test_id(&mut runtime, &mut tree, "framework-combobox", "tab");
        assert!(!tree.elements.values().any(|element| {
            element.test_id.as_deref() == Some("framework-combobox-option-Rust")
        }));
        dispatch_test_id(
            &mut runtime,
            &mut tree,
            "framework-combobox",
            "change",
            Some("ru"),
        );
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

    fn has_foreground_box_shadow(tree: &RetainedTree, test_id: &str) -> bool {
        tree.elements
            .values()
            .find(|element| element.test_id.as_deref() == Some(test_id))
            .and_then(|element| element.style.as_ref())
            .and_then(|style| style.foreground_box_shadow.as_ref())
            .is_some_and(|shadow| shadow.inset)
    }
}
