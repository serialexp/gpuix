//! `gpx` for JavaScript apps: QuickJS runs the app, and GPUI owns the window
//! and the run loop. The app's `render()` passes its window options to the
//! embedded `@gpuix/native` shim's `init`, so the app loads before the
//! window opens.
//!
//! As in `lua_app.rs`, the window's events go through a channel to one task
//! on GPUI's foreground executor, so JS never runs inside a GPUI update. The
//! same task runs the app's timers when they are due, and reloads the app
//! when `--watch` sees a source change. Once the window opens, the app's
//! renderer reaches it through a [`WindowHost`].

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::mpsc::UnboundedReceiver;
use futures::{FutureExt as _, StreamExt as _};

use crate::element_tree::EventPayload;
use crate::quickjs::{
    HotUpdate, Mode, PathPromptResult, QuickJsOptions, QuickJsRuntime, SizeOverride, TreeHost,
    WindowCall,
};
use crate::quickjs_window::WindowHost;
use crate::renderer::{init_app, open_main_window, EventCallback, GpuixView, WindowOptions};
use crate::script::{default_title, reload, AppOptions};
use crate::text::SharedSelection;

/// The extensions of the files the QuickJS runtime loads.
pub const JS_EXTENSIONS: [&str; 6] = crate::quickjs::EXTENSIONS;

/// At most this many rounds of zero-delay timers run before the window
/// opens. React's scheduler commits the first tree in zero-delay slices of
/// about 5 ms, so this leaves it seconds. An app that keeps queuing
/// `setImmediate` still opens, and its timers go on in the window's loop.
const STARTUP_TIMER_ROUNDS: usize = 1_000;

/// Whether `path` is a module the QuickJS runtime loads, which `gpx` runs
/// as a JavaScript app. Type declarations (`.d.ts`) hold no code.
pub fn is_js_source(path: &Path) -> bool {
    let declaration = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".d.ts"));
    !declaration
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| JS_EXTENSIONS.contains(&extension))
}

/// Loads the app at `path` as `gpx` would, without opening a window: it
/// fails if a module can't be found or compiled, the app throws while it
/// starts, or it never calls `render()`.
pub fn check_js_file(path: impl AsRef<Path>) -> Result<(), String> {
    start(path.as_ref(), SizeOverride::default(), Mode::Production)?;
    Ok(())
}

/// Runs the app at `path` in a window until the window closes.
pub fn run_js_file(path: impl AsRef<Path>, options: AppOptions) -> Result<(), String> {
    let dev_server = crate::script::server::Server::start(options.dev_server)?;
    let path = path.as_ref().to_path_buf();
    // `--watch` runs the frameworks' development builds, for React Fast
    // Refresh.
    let mode = if options.watch {
        Mode::Development
    } else {
        Mode::Production
    };
    let (js, app_window_options) = start(&path, command_line_size(&options), mode)?;
    let window_options = window_options(app_window_options, &path, &options);
    let startup_error = Rc::new(RefCell::new(None));
    let startup_error_for_app = startup_error.clone();

    gpui_platform::application()
        .with_quit_mode(gpui::QuitMode::LastWindowClosed)
        .run(move |cx| {
            let app_name = window_options
                .app_name
                .clone()
                .unwrap_or_else(|| window_options.title_or_default());
            init_app(&app_name, cx);
            if let Some(server) = dev_server {
                server.attach(cx);
            }
            cx.on_window_closed(|_cx, native_id| crate::text::paint::remove_window(native_id))
                .detach();
            let reloads = options.watch.then(|| watch_sources(&path));
            match launch(path, js, &window_options, reloads, cx) {
                Ok((_window, task)) => task.detach(),
                Err(error) => {
                    *startup_error_for_app.borrow_mut() = Some(error);
                    cx.quit();
                }
            }
        });

    let error = startup_error.borrow_mut().take();
    match error {
        Some(error) => Err(format!("Failed to open the GPUI window: {error}")),
        None => Ok(()),
    }
}

/// Opens the window for a started app, and returns it with the task that
/// runs the app in it until the window closes.
pub(crate) fn launch(
    path: PathBuf,
    js: QuickJsRuntime,
    window_options: &WindowOptions,
    reloads: Option<UnboundedReceiver<Vec<PathBuf>>>,
    cx: &mut gpui::App,
) -> Result<(gpui::WindowHandle<GpuixView>, gpui::Task<()>), String> {
    let tree = Arc::new(Mutex::new(js.with_tree(std::mem::take)));
    let selection = SharedSelection::default();
    let (callback, events) = event_channel();
    let window = open_main_window(
        window_options,
        tree.clone(),
        Some(callback),
        selection.clone(),
        cx,
    )?;
    let (prompt_results, prompts) = futures::channel::mpsc::unbounded();
    let startup = js.set_host(Box::new(WindowHost::new(
        tree,
        window,
        selection,
        prompt_results,
        cx.to_async(),
    )));
    // The new window draws the tree as it stands.
    js.take_tree_changed();
    let app = RunningApp {
        path,
        js,
        window,
        title: window_options.title_or_default(),
    };
    let pending = startup.into_pending();
    let task = cx.spawn(async move |cx| app.run(pending, events, prompts, reloads, cx).await);
    Ok((window, task))
}

/// Imports the app and runs the timers due straight away, so `render()` has
/// committed its first tree, then returns the window options `render()`
/// passed. Until the window opens, the app sees the size it will open at,
/// with `size` on top of the app's. Fails if the app throws or never calls
/// `render()`.
pub(crate) fn start(
    path: &Path,
    size: SizeOverride,
    mode: Mode,
) -> Result<(QuickJsRuntime, WindowOptions), String> {
    let options = QuickJsOptions {
        host: Box::new(TreeHost::new(size)),
        mode,
        ..QuickJsOptions::default()
    };
    let mut js = QuickJsRuntime::load_module(path, options)?;
    for _ in 0..STARTUP_TIMER_ROUNDS {
        if js.next_timer()? != Some(Duration::ZERO) {
            break;
        }
        js.run_timers()?;
    }
    let options = js
        .window_options()
        .ok_or_else(|| format!("{} never called render() while it started", path.display()))?;
    Ok((js, options))
}

/// The app's window options, with the command line's on top, and the entry
/// file's name as the title when neither sets one.
fn window_options(mut options: WindowOptions, entry: &Path, app: &AppOptions) -> WindowOptions {
    if let Some(title) = &app.title {
        options.title = Some(title.clone());
    }
    options.title.get_or_insert_with(|| default_title(entry));
    command_line_size(app).apply(&mut options);
    options
}

fn command_line_size(app: &AppOptions) -> SizeOverride {
    SizeOverride {
        width: app.width.map(f64::from),
        height: app.height.map(f64::from),
    }
}

/// A view's event callback, and where its events arrive. Each view gets a
/// channel of its own: closing the window drops the only sender, which ends
/// the app's task, and a reloaded app never sees events for the old tree's
/// element ids.
fn event_channel() -> (EventCallback, UnboundedReceiver<EventPayload>) {
    let (sender, receiver) = futures::channel::mpsc::unbounded();
    let callback: EventCallback = Arc::new(move |payload| {
        // The receiver goes when the app stops or reloads.
        sender.unbounded_send(payload).ok();
    });
    (callback, receiver)
}

struct RunningApp {
    path: PathBuf,
    js: QuickJsRuntime,
    window: gpui::WindowHandle<GpuixView>,
    title: String,
}

/// Where a reloaded app's events and path prompt results arrive.
type Reloaded = (
    UnboundedReceiver<EventPayload>,
    UnboundedReceiver<(u64, PathPromptResult)>,
);

/// What woke the app's task. The event is boxed: it is far larger than the
/// other variants.
enum Wake {
    Event(Option<Box<EventPayload>>),
    PathPrompt(Option<(u64, PathPromptResult)>),
    Timer,
    SourceChange(Option<Vec<PathBuf>>),
}

impl RunningApp {
    /// Runs the window methods the app called before its window opened,
    /// then delivers events, runs timers and reloads until the window
    /// closes.
    async fn run(
        mut self,
        pending: Vec<WindowCall>,
        mut events: UnboundedReceiver<EventPayload>,
        mut prompts: UnboundedReceiver<(u64, PathPromptResult)>,
        mut reloads: Option<UnboundedReceiver<Vec<PathBuf>>>,
        cx: &mut gpui::AsyncApp,
    ) {
        self.replay(pending);
        loop {
            let timer = match self.js.next_timer() {
                // GPUI's timer is ready at once for no delay, and an app that
                // keeps queuing zero-delay timers would then never let GPUI
                // paint or deliver input.
                Ok(Some(Duration::ZERO)) => yield_now().boxed_local(),
                Ok(Some(delay)) => cx.background_executor().timer(delay).boxed_local(),
                Ok(None) => futures::future::pending().boxed_local(),
                Err(error) => {
                    // Only a broken prelude gets here (an app that patched a
                    // builtin it uses), and the app's timers can't run again.
                    eprintln!("gpx: {error}; the app stopped");
                    self.window
                        .update(cx, |_view, window, _cx| window.remove_window())
                        .ok();
                    return;
                }
            };
            let wake = {
                let source_change = async {
                    match &mut reloads {
                        Some(reloads) => reloads.next().await,
                        None => futures::future::pending().await,
                    }
                };
                futures::select_biased! {
                    event = events.next() => Wake::Event(event.map(Box::new)),
                    prompt = prompts.next() => Wake::PathPrompt(prompt),
                    _ = timer.fuse() => Wake::Timer,
                    change = source_change.fuse() => Wake::SourceChange(change),
                }
            };
            match wake {
                // The window closed, which dropped the view and its sender.
                Wake::Event(None) => return,
                Wake::Event(Some(event)) => self.dispatch(&event),
                Wake::PathPrompt(Some((id, result))) => {
                    if let Err(error) = self.js.settle_path_prompt(id, result) {
                        eprintln!("gpx: {error}");
                    }
                }
                // The host holds a sender for as long as the app runs.
                Wake::PathPrompt(None) => {}
                Wake::Timer => self.run_timers(),
                Wake::SourceChange(Some(changed)) => {
                    if self.hot_update(&changed) {
                        if let Some((reloaded_events, reloaded_prompts)) = self.reload(cx) {
                            events = reloaded_events;
                            prompts = reloaded_prompts;
                        }
                    }
                }
                // The watcher stopped; it said why.
                Wake::SourceChange(None) => reloads = None,
            }
            if self.js.take_tree_changed() {
                self.window
                    .update(cx, |_view, _window, cx| cx.notify())
                    .ok();
            }
        }
    }

    fn dispatch(&mut self, event: &EventPayload) {
        if let Err(error) = self.js.dispatch_event(event) {
            eprintln!("gpx: {error}");
        }
    }

    fn run_timers(&mut self) {
        if let Err(error) = self.js.run_timers() {
            eprintln!("gpx: {error}");
        }
    }

    /// Runs window methods the app called before there was a window to
    /// run them. A failure is reported here, as nothing in the app awaits it.
    fn replay(&self, pending: Vec<WindowCall>) {
        for error in self.js.replay(pending) {
            eprintln!("gpx: {error}");
        }
    }

    /// Updates the app in place for the files at `changed`, if it can, and
    /// says whether it has to start again instead. A file that fails to
    /// compile or run leaves the app as it was, until the next change.
    fn hot_update(&mut self, changed: &[PathBuf]) -> bool {
        match self.js.hot_update(changed) {
            Ok(HotUpdate::Refreshed(files)) => {
                let names: Vec<String> = files.iter().map(|file| self.display(file)).collect();
                eprintln!("gpx: hot updated {}", names.join(", "));
                false
            }
            Ok(HotUpdate::Unchanged) => false,
            Ok(HotUpdate::Restart(reason)) => {
                eprintln!("gpx: {reason}, so the app starts again");
                true
            }
            Err(error) => {
                eprintln!("gpx: hot update failed: {error}");
                false
            }
        }
    }

    /// `file` relative to the entry file's directory, when it is in it.
    fn display(&self, file: &str) -> String {
        let directory = std::fs::canonicalize(&self.path)
            .ok()
            .and_then(|entry| entry.parent().map(Path::to_path_buf));
        directory
            .and_then(|directory| {
                Path::new(file)
                    .strip_prefix(directory)
                    .ok()
                    .map(Path::to_path_buf)
            })
            .map_or_else(
                || file.to_string(),
                |relative| relative.display().to_string(),
            )
    }

    /// Starts the app again in a new runtime, and swaps it in with a new
    /// view, so nothing the old tree's element ids keyed carries over. The
    /// window and its options stay; the app's state is reset. Returns where
    /// the new view's events and the new app's path prompts arrive, or
    /// `None` if the old app stays.
    fn reload(&mut self, cx: &mut gpui::AsyncApp) -> Option<Reloaded> {
        // The window keeps its size, which the user may have changed.
        let size = self
            .window
            .update(cx, |_view, window, _cx| {
                let size = window.viewport_size();
                SizeOverride {
                    width: Some(f64::from(f32::from(size.width))),
                    height: Some(f64::from(f32::from(size.height))),
                }
            })
            .unwrap_or_default();
        let js = match start(&self.path, size, self.js.mode()) {
            Ok((js, _window_options)) => js,
            Err(error) => {
                eprintln!("gpx: reload failed: {error}");
                return None;
            }
        };
        let tree = Arc::new(Mutex::new(js.with_tree(std::mem::take)));
        let selection = SharedSelection::default();
        let (callback, events) = event_channel();
        let (view_tree, view_selection, title) =
            (tree.clone(), selection.clone(), self.title.clone());
        let swapped = gpui::AnyWindowHandle::from(self.window).update(cx, |_view, window, cx| {
            window.replace_root(cx, |_window, _cx| {
                GpuixView::new(view_tree, Some(callback), title, view_selection)
            });
        });
        if let Err(error) = swapped {
            eprintln!("gpx: reload failed: {error}");
            return None;
        }
        let (prompt_results, prompts) = futures::channel::mpsc::unbounded();
        let startup = js.set_host(Box::new(WindowHost::new(
            tree,
            self.window,
            selection,
            prompt_results,
            cx.clone(),
        )));
        // The new view draws the tree as it starts.
        js.take_tree_changed();
        self.js = js;
        self.replay(startup.into_pending());
        eprintln!("gpx: reloaded (state reset)");
        Some((events, prompts))
    }
}

/// Ready the second time it is polled: the task goes to the back of the
/// queue once, behind GPUI's own work.
fn yield_now() -> impl std::future::Future<Output = ()> {
    let mut yielded = false;
    futures::future::poll_fn(move |cx| {
        if yielded {
            std::task::Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    })
}

fn watch_sources(entry: &Path) -> UnboundedReceiver<Vec<PathBuf>> {
    reload::watch_sources(entry, is_js_source, "gpx")
}

#[cfg(test)]
mod tests {
    use futures::channel::mpsc::UnboundedSender;

    use super::*;

    /// A directory with one app file, removed when dropped.
    struct App(PathBuf);

    impl App {
        fn new(name: &str, source: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "gpx-app-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(name), source).unwrap();
            Self(dir.join(name))
        }

        fn write(&self, source: &str) {
            std::fs::write(&self.0, source).unwrap();
        }

        /// Writes `source` to `name`, next to the entry file.
        fn write_file(&self, name: &str, source: &str) {
            std::fs::write(self.0.parent().unwrap().join(name), source).unwrap();
        }
    }

    impl Drop for App {
        fn drop(&mut self) {
            std::fs::remove_dir_all(self.0.parent().unwrap()).ok();
        }
    }

    /// A counter whose clicks also set a label from a zero-delay timer.
    fn counter(label: &str) -> String {
        format!(
            r#"import {{ render }} from "@gpuix/react"
            import {{ useState }} from "react"
            function Counter() {{
              const [count, setCount] = useState(0)
              const [later, setLater] = useState("")
              const click = () => {{
                setCount(count + 1)
                setTimeout(() => setLater("timer ran"), 0)
              }}
              return (
                <div testId="button" onClick={{click}}>
                  <text testId="count">{{`{label} ${{count}}`}}</text>
                  <text testId="later">{{later}}</text>
                </div>
              )
            }}
            render(<Counter />, {{ title: "Counter", focus: false }})"#
        )
    }

    /// `Counter` in a file of its own, for a hot update to change.
    fn component(label: &str) -> String {
        format!(
            r#"import {{ useState }} from "react"
            export function Counter() {{
              const [count, setCount] = useState(0)
              return (
                <div testId="button" onClick={{() => setCount(count + 1)}}>
                  <text testId="count">{{`{label} ${{count}}`}}</text>
                </div>
              )
            }}"#
        )
    }

    const COMPONENT_ENTRY: &str = r#"import { render } from "@gpuix/react"
        import { Counter } from "./counter"
        render(<Counter />, { title: "Counter", focus: false })"#;

    /// An app `launch`ed on a GPUI test app, with a reload channel the test
    /// drives.
    struct Launched {
        cx: gpui::TestAppContext,
        window: gpui::WindowHandle<GpuixView>,
        task: Option<gpui::Task<()>>,
        reload: UnboundedSender<Vec<PathBuf>>,
        entry: PathBuf,
    }

    impl Launched {
        fn new(app: &App) -> Self {
            Self::in_mode(app, Mode::Production)
        }

        fn in_mode(app: &App, mode: Mode) -> Self {
            let cx = gpui::TestAppContext::single();
            let (js, options) = start(&app.0, SizeOverride::default(), mode).unwrap();
            let (reload, reloads) = futures::channel::mpsc::unbounded();
            let (window, task) = cx
                .update(|cx| launch(app.0.clone(), js, &options, Some(reloads), cx))
                .unwrap();
            Self {
                cx,
                window,
                task: Some(task),
                reload,
                entry: app.0.clone(),
            }
        }

        fn view<R>(&mut self, read: impl FnOnce(&GpuixView) -> R) -> R {
            self.window
                .update(&mut self.cx, |view, _window, _cx| read(view))
                .unwrap()
        }

        /// The text of the element with `test_id` and its descendants.
        fn text(&mut self, test_id: &str) -> String {
            self.view(|view| {
                let tree = view.tree.lock().unwrap();
                let element = tree
                    .elements
                    .values()
                    .find(|element| element.test_id.as_deref() == Some(test_id))
                    .unwrap_or_else(|| panic!("no element has testId {test_id:?}"));
                let mut text = String::new();
                let mut pending = vec![element.id];
                while let Some(id) = pending.pop() {
                    let element = &tree.elements[&id];
                    text.push_str(element.content.as_deref().unwrap_or(""));
                    pending.extend(element.children.iter().rev());
                }
                text
            })
        }

        /// The current view's event callback.
        fn callback(&mut self) -> EventCallback {
            self.view(|view| view.event_callback.clone().unwrap())
        }

        /// Clicks the element with `test_id` through `callback`, as the view
        /// would, and lets the app's task run.
        fn click_with(&mut self, callback: &EventCallback, test_id: &str) {
            let element_id = self.element_id(test_id);
            callback(click(element_id));
            self.cx.run_until_parked();
        }

        fn element_id(&mut self, test_id: &str) -> u64 {
            self.view(|view| {
                let tree = view.tree.lock().unwrap();
                tree.elements
                    .values()
                    .find(|element| element.test_id.as_deref() == Some(test_id))
                    .map(|element| element.id)
                    .unwrap()
            })
        }

        fn click(&mut self, test_id: &str) {
            let callback = self.callback();
            self.click_with(&callback, test_id);
        }

        /// Reports that the entry file changed, as the watcher would.
        fn reload(&mut self) {
            let name = self
                .entry
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            self.changed(&[&name]);
        }

        /// Reports that the files `names`, next to the entry, changed.
        fn changed(&mut self, names: &[&str]) {
            let directory = self.entry.parent().unwrap();
            let paths = names.iter().map(|name| directory.join(name)).collect();
            self.reload.unbounded_send(paths).unwrap();
            self.cx.run_until_parked();
        }

        fn finished(&mut self) -> bool {
            self.task.take().unwrap().now_or_never().is_some()
        }
    }

    fn click(element_id: u64) -> EventPayload {
        EventPayload {
            element_id: element_id as f64,
            event_type: "click".to_string(),
            ..EventPayload::default()
        }
    }

    impl Drop for Launched {
        fn drop(&mut self) {
            self.cx.quit();
        }
    }

    #[test]
    fn check_loads_an_app_that_renders() {
        let app = App::new(
            "main.tsx",
            r#"import { render } from "@gpuix/react"
            render(<text>ok</text>, { title: "Checked" })"#,
        );
        check_js_file(&app.0).unwrap();
    }

    #[test]
    fn check_fails_an_app_that_never_renders_or_throws() {
        let app = App::new("main.ts", "export const value = 1");
        let error = check_js_file(&app.0).err().unwrap();
        assert!(
            error.ends_with("main.ts never called render() while it started"),
            "{error}"
        );
        let app = App::new("main.ts", "throw new Error('broken')");
        let error = check_js_file(&app.0).err().unwrap();
        assert!(error.contains("Error: broken"), "{error}");
    }

    #[test]
    fn an_app_that_keeps_queuing_immediates_still_starts() {
        let app = App::new(
            "main.tsx",
            r#"import { render } from "@gpuix/react"
            const spin = () => setImmediate(spin)
            spin()
            render(<text>ok</text>)"#,
        );
        check_js_file(&app.0).unwrap();
    }

    #[test]
    fn the_command_line_beats_the_apps_window_options() {
        let app = WindowOptions {
            title: Some("App".into()),
            width: Some(300.0),
            height: Some(200.0),
            ..WindowOptions::default()
        };
        let entry = Path::new("app/main.tsx");
        let options = window_options(
            app.clone(),
            entry,
            &AppOptions {
                title: Some("Command line".into()),
                width: Some(640.0),
                ..AppOptions::default()
            },
        );
        assert_eq!(options.title.as_deref(), Some("Command line"));
        assert_eq!(options.width, Some(640.0));
        assert_eq!(options.height, Some(200.0));
        let options = window_options(app.clone(), entry, &AppOptions::default());
        assert_eq!(options.title.as_deref(), Some("App"));
        let untitled = WindowOptions { title: None, ..app };
        let options = window_options(untitled, entry, &AppOptions::default());
        assert_eq!(options.title.as_deref(), Some("main"));
    }

    #[test]
    fn only_app_modules_trigger_a_reload() {
        for name in ["a.tsx", "a.ts", "a.mts", "a.jsx", "a.js", "a.mjs"] {
            assert!(is_js_source(Path::new(name)), "{name}");
        }
        for name in ["a.lua", "a.json", "a.d", "a", "a.TSX", "a.d.ts"] {
            assert!(!is_js_source(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn events_and_timers_reach_the_running_app() {
        let app = App::new("main.tsx", &counter("Count"));
        let mut launched = Launched::new(&app);
        assert_eq!(launched.text("count"), "Count 0");
        launched.click("button");
        assert_eq!(launched.text("count"), "Count 1");
        assert_eq!(launched.text("later"), "timer ran");
    }

    #[test]
    fn a_reload_starts_the_app_fresh_and_drops_the_old_views_events() {
        let app = App::new("main.tsx", &counter("Count"));
        let mut launched = Launched::new(&app);
        launched.click("button");
        let old_callback = launched.callback();
        app.write(&counter("Clicks"));
        launched.reload();
        assert_eq!(launched.text("count"), "Clicks 0");
        // An event the old view sent lands nowhere.
        launched.click_with(&old_callback, "button");
        assert_eq!(launched.text("count"), "Clicks 0");
        launched.click("button");
        assert_eq!(launched.text("count"), "Clicks 1");
    }

    #[test]
    fn a_watched_app_updates_a_component_in_place() {
        let app = App::new("main.tsx", COMPONENT_ENTRY);
        app.write_file("counter.tsx", &component("Count"));
        let mut launched = Launched::in_mode(&app, Mode::Development);
        launched.click("button");
        let callback = launched.callback();
        app.write_file("counter.tsx", &component("Clicks"));
        launched.changed(&["counter.tsx"]);
        assert_eq!(launched.text("count"), "Clicks 1");
        // The same view: its events still reach the app.
        launched.click_with(&callback, "button");
        assert_eq!(launched.text("count"), "Clicks 2");
    }

    #[test]
    fn a_watched_app_starts_again_when_its_entry_changes() {
        let app = App::new("main.tsx", COMPONENT_ENTRY);
        app.write_file("counter.tsx", &component("Count"));
        let mut launched = Launched::in_mode(&app, Mode::Development);
        launched.click("button");
        app.write(&format!(
            "{COMPONENT_ENTRY}
// changed"
        ));
        launched.reload();
        assert_eq!(launched.text("count"), "Count 0");
        launched.click("button");
        assert_eq!(launched.text("count"), "Count 1");
    }

    #[test]
    fn a_reload_that_fails_or_never_renders_keeps_the_running_app() {
        let app = App::new("main.tsx", &counter("Count"));
        let mut launched = Launched::new(&app);
        launched.click("button");
        for broken in [
            "const = 1",
            "export const value = 1",
            "throw new Error('broken')",
        ] {
            app.write(broken);
            launched.reload();
            assert_eq!(launched.text("count"), "Count 1", "{broken}");
        }
        launched.click("button");
        assert_eq!(launched.text("count"), "Count 2");
    }

    #[test]
    fn closing_the_window_ends_the_app() {
        let app = App::new("main.tsx", &counter("Count"));
        let mut launched = Launched::new(&app);
        launched
            .window
            .update(&mut launched.cx, |_view, window, _cx| {
                window.remove_window()
            })
            .unwrap();
        launched.cx.run_until_parked();
        assert!(launched.finished());
    }

    #[test]
    fn an_app_that_keeps_queuing_immediates_still_takes_events() {
        let app = App::new(
            "main.tsx",
            r#"import { render } from "@gpuix/react"
            import { useState } from "react"
            const spin = () => setImmediate(spin)
            spin()
            function Counter() {
              const [count, setCount] = useState(0)
              return <text testId="count" onClick={() => setCount(count + 1)}>{`Count ${count}`}</text>
            }
            render(<Counter />)"#,
        );
        let mut launched = Launched::new(&app);
        let callback = launched.callback();
        let element_id = launched.element_id("count");
        callback(click(element_id));
        // The app's task never parks, so step the executor instead of
        // running it until it does.
        for _ in 0..20 {
            launched.cx.executor().tick();
        }
        assert_eq!(launched.text("count"), "Count 1");
    }

    #[test]
    fn the_app_runs_on_after_its_watcher_stops() {
        let app = App::new("main.tsx", &counter("Count"));
        let mut launched = Launched::new(&app);
        launched.reload.close_channel();
        launched.cx.run_until_parked();
        launched.click("button");
        assert_eq!(launched.text("count"), "Count 1");
    }

    /// Two focusable boxes. The app focuses the first as it mounts, before
    /// its window opens; `button` reports the window's size and focuses the
    /// second, and `next` moves focus on. `first-size` is the size the
    /// first render saw.
    const WINDOW_APP: &str = r#"import { render, useGpuix } from "@gpuix/react"
        import { useEffect, useRef, useState } from "react"
        function App() {
          const { renderer } = useGpuix()
          const first = useRef<any>(null)
          const second = useRef<any>(null)
          const [info, setInfo] = useState("")
          const [firstSize] = useState(() => {
            const size = renderer!.getWindowSize!()
            return `${size.width}x${size.height}`
          })
          useEffect(() => {
            renderer!.setWindowTitle!("Mounted")
            renderer!.focusElement!(first.current.id)
          }, [])
          const report = () => {
            const size = renderer!.getWindowSize!()
            renderer!.focusElement!(second.current.id)
            setInfo(`${size.width}x${size.height}`)
          }
          return (
            <div>
              <div testId="first" ref={first} tabIndex={0} />
              <div testId="second" ref={second} tabIndex={0} />
              <div testId="button" onClick={report}><text testId="info">{info}</text></div>
              <div testId="next" onClick={() => renderer!.focusNext!()} />
              <text testId="first-size">{firstSize}</text>
            </div>
          )
        }
        render(<App />, { title: "Window", width: 320, height: 240, focus: false })"#;

    impl Launched {
        fn focused(&mut self) -> Option<u64> {
            self.window
                .update(&mut self.cx, |view, window, _cx| {
                    view.focused_element_id(window)
                })
                .unwrap()
        }

        fn title(&mut self) -> String {
            self.view(|view| view.window_title.clone())
        }
    }

    #[test]
    fn what_the_app_asks_of_its_window_before_it_opens_happens_once_it_has() {
        let app = App::new("main.tsx", WINDOW_APP);
        let mut launched = Launched::new(&app);
        launched.cx.run_until_parked();
        assert_eq!(launched.title(), "Mounted");
        let first = launched.element_id("first");
        assert_eq!(launched.focused(), Some(first));
        // As under Node, where `init` opens the window before the first
        // render.
        assert_eq!(launched.text("first-size"), "320x240");
    }

    #[test]
    fn the_app_reaches_its_open_window() {
        let app = App::new("main.tsx", WINDOW_APP);
        let mut launched = Launched::new(&app);
        launched.cx.run_until_parked();
        launched.click("button");
        assert_eq!(launched.text("info"), "320x240");
        let second = launched.element_id("second");
        assert_eq!(launched.focused(), Some(second));
        launched.click("next");
        let next = launched.focused();
        assert!(next.is_some() && next != Some(second), "{next:?}");
    }

    #[test]
    fn a_reloaded_app_reaches_the_window_too() {
        let app = App::new("main.tsx", WINDOW_APP);
        let mut launched = Launched::new(&app);
        launched.cx.run_until_parked();
        launched.cx.simulate_window_resize(
            launched.window.into(),
            gpui::size(gpui::px(400.0), gpui::px(300.0)),
        );
        launched.reload();
        // The new app's tree has new ids, and it focused its own first box.
        let first = launched.element_id("first");
        assert_eq!(launched.focused(), Some(first));
        // It starts in the window as it is now, not as the app asks.
        assert_eq!(launched.text("first-size"), "400x300");
        launched.click("button");
        assert_eq!(launched.text("info"), "400x300");
    }

    /// Opens a path prompt on a click, and shows what it settled with.
    const PROMPT_APP: &str = r#"import { render, useGpuix } from "@gpuix/react"
        import { useState } from "react"
        function App() {
          const { renderer } = useGpuix()
          const [picked, setPicked] = useState("none")
          const open = () => {
            renderer!.promptForPaths!({ multiple: true }).then(
              (paths) => setPicked(JSON.stringify(paths)),
              (error) => setPicked(String(error)),
            )
          }
          return <div testId="open" onClick={open}><text testId="picked">{picked}</text></div>
        }
        render(<App />, { title: "Prompt", focus: false })"#;

    #[test]
    fn a_path_prompt_resolves_with_what_the_picker_picked() {
        let app = App::new("main.tsx", PROMPT_APP);
        let mut launched = Launched::new(&app);
        launched.click("open");
        assert!(launched.cx.did_prompt_for_paths());
        launched.cx.simulate_path_prompt_response(|options| {
            assert!(options.files && options.multiple);
            Some(vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")])
        });
        launched.cx.run_until_parked();
        assert_eq!(launched.text("picked"), r#"["/tmp/a","/tmp/b"]"#);
        launched.click("open");
        launched.cx.simulate_path_prompt_response(|_| None);
        launched.cx.run_until_parked();
        assert_eq!(launched.text("picked"), "null");
    }

    #[test]
    fn a_prompt_the_old_app_opened_doesnt_reach_the_reloaded_one() {
        let app = App::new("main.tsx", PROMPT_APP);
        let mut launched = Launched::new(&app);
        launched.click("open");
        launched.reload();
        launched
            .cx
            .simulate_path_prompt_response(|_| Some(vec![PathBuf::from("/tmp/a")]));
        launched.cx.run_until_parked();
        assert_eq!(launched.text("picked"), "none");
    }
}
