use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use gpui::AppContext;
use serde::Serialize;

use crate::lua_runtime::LuaRuntime;
use crate::quickjs::{Mode, QuickJsRuntime, SizeOverride};
use crate::renderer::{GpuixView, WindowOptions};
use crate::retained_tree::RetainedTree;

enum Interpreter {
    Lua { _runtime: LuaRuntime },
    JavaScript { _runtime: QuickJsRuntime },
}

struct Prepared {
    interpreter: Option<Interpreter>,
    tree: Arc<Mutex<RetainedTree>>,
    options: WindowOptions,
    lua: bool,
    load_ms: f64,
    mount_ms: f64,
}

fn elapsed(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn prepare(path: &Path) -> Result<Prepared, String> {
    let start = Instant::now();
    if crate::quickjs_app::is_js_source(path) {
        let (runtime, mut options) =
            crate::quickjs_app::start(path, SizeOverride::default(), Mode::Development)?;
        let load_ms = elapsed(start);
        options.focus = Some(false);
        Ok(Prepared {
            tree: Arc::new(Mutex::new(runtime.with_tree(std::mem::take))),
            interpreter: Some(Interpreter::JavaScript { _runtime: runtime }),
            options,
            lua: false,
            load_ms,
            mount_ms: 0.0,
        })
    } else {
        let mut runtime = LuaRuntime::load_application_file(path)?;
        let options = runtime
            .window_options()
            .into_iter()
            .find(|window| window.open)
            .ok_or("benchmark app must initially open a window")?;
        let load_ms = elapsed(start);
        let start = Instant::now();
        let mut tree = RetainedTree::new();
        runtime.mount_window(&options.id, &mut tree)?;
        Ok(Prepared {
            tree: Arc::new(Mutex::new(tree)),
            interpreter: Some(Interpreter::Lua { _runtime: runtime }),
            options: WindowOptions {
                width: Some(f64::from(options.width)),
                height: Some(f64::from(options.height)),
                title: Some(options.title),
                focus: Some(false),
                ..Default::default()
            },
            lua: true,
            load_ms,
            mount_ms: elapsed(start),
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Sample {
    load_ms: f64,
    mount_ms: f64,
    dispose_ms: f64,
    replace_view_ms: f64,
    draw_ms: f64,
    total_ms: f64,
    nodes: usize,
}

fn draw(window: gpui::WindowHandle<GpuixView>, cx: &mut gpui::App) -> Result<f64, String> {
    let start = Instant::now();
    gpui::AnyWindowHandle::from(window)
        .update(cx, |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .map_err(|error| error.to_string())?;
    Ok(elapsed(start))
}

pub fn benchmark_script_resets(
    path: &Path,
    iterations: usize,
) -> Result<serde_json::Value, String> {
    if !(1..=1000).contains(&iterations) {
        return Err("iterations must be between 1 and 1000".into());
    }
    let path = std::fs::canonicalize(path).map_err(|error| error.to_string())?;
    if !crate::quickjs_app::is_js_source(&path) && !crate::lua_app::is_lua_source(&path) {
        return Err("benchmark expects a Lua/LuaX or JavaScript/TypeScript entry".into());
    }
    let start = Instant::now();
    let mut prepared = prepare(&path)?;
    let result = Rc::new(RefCell::new(None));
    let output = result.clone();
    gpui_platform::application().run(move |cx| {
        let mut run = || -> Result<serde_json::Value, String> {
            crate::renderer::init_app("GPUIX reset benchmark", cx);
            if prepared.lua { crate::renderer::init_lua_focus_key_bindings(cx); }
            let native_start = Instant::now();
            let (width, height) = prepared.options.size();
            let bounds = gpui::Bounds::centered(None, gpui::size(gpui::px(width as f32), gpui::px(height as f32)), cx);
            let tree = prepared.tree.clone();
            let lua = prepared.lua;
            let window = cx.open_window(crate::renderer::to_gpui_window_options(&prepared.options, bounds), |_, cx| cx.new(|_| {
                let view = GpuixView::new(tree, None, "GPUIX reset benchmark".into(), Default::default());
                if lua { view.with_native_tab_navigation() } else { view }
            })).map_err(|error| error.to_string())?;
            let window_ms = elapsed(native_start);
            let first_draw_ms = draw(window, cx)?;
            let initial_ms = elapsed(start);
            eprintln!("gpx: reset-bench initial scene ready");
            let initial = serde_json::json!({"loadMs":prepared.load_ms, "mountMs":prepared.mount_ms,
                "windowMs":window_ms, "drawMs":first_draw_ms, "totalMs":initial_ms,
                "nativeInitAndOtherMs":initial_ms - prepared.load_ms - prepared.mount_ms - window_ms - first_draw_ms,
                "nodes":prepared.tree.lock().unwrap().elements.len()});
            let mut samples = Vec::new();
            for _ in 0..iterations {
                let start = Instant::now();
                let dispose = Instant::now();
                drop(prepared.interpreter.take());
                let dispose_ms = elapsed(dispose);
                let fresh = prepare(&path)?;
                let replace = Instant::now();
                let tree = fresh.tree.clone();
                let lua = fresh.lua;
                gpui::AnyWindowHandle::from(window).update(cx, |_, window, cx| {
                    window.replace_root(cx, |_, _| {
                        let view = GpuixView::new(tree, None, "GPUIX reset benchmark".into(), Default::default());
                        if lua { view.with_native_tab_navigation() } else { view }
                    });
                }).map_err(|error| error.to_string())?;
                prepared = fresh;
                let replace_view_ms = elapsed(replace);
                let draw_ms = draw(window, cx)?;
                samples.push(Sample { load_ms:prepared.load_ms, mount_ms:prepared.mount_ms, dispose_ms,
                    replace_view_ms, draw_ms, total_ms:elapsed(start), nodes:prepared.tree.lock().unwrap().elements.len() });
            }
            Ok(serde_json::json!({"initial":initial, "resets":samples,
                "javascriptLoadIncludesMount": !prepared.lua,
                "windowId":window.window_id().as_u64(), "windowCount":cx.windows().len(),
                "scope":"First open window; fresh interpreter and view each reset; CPU scene build only, not GPU completion; no post-mount events, HTTP completion or timers"}))
        };
        let value = run();
        match &value {
            Ok(report) => println!("{}", serde_json::to_string_pretty(report).unwrap()),
            Err(error) => {
                eprintln!("gpx: reset benchmark failed: {error}");
                std::process::exit(1);
            }
        }
        *output.borrow_mut() = Some(value);
        cx.quit();
    });
    let value = result
        .borrow_mut()
        .take()
        .ok_or("benchmark application did not start")?;
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preparation_reconstructs_the_same_lua_tree_in_fresh_interpreters() {
        let directory = tempfile::tempdir().unwrap();
        let entry = directory.path().join("main.luax");
        std::fs::write(&entry, "return function() return <text>Initial</text> end").unwrap();
        let first = prepare(&entry).unwrap();
        let first_tree = first
            .tree
            .lock()
            .unwrap()
            .to_automation_json(&Default::default());
        drop(first);
        let fresh = prepare(&entry).unwrap();
        assert_eq!(
            first_tree,
            fresh
                .tree
                .lock()
                .unwrap()
                .to_automation_json(&Default::default())
        );
        assert!(!fresh.options.focus.unwrap());
        assert!(fresh.lua);
    }

    #[test]
    fn benchmark_rejects_invalid_runs_before_starting_a_native_app() {
        assert!(benchmark_script_resets(Path::new("missing.lua"), 0).is_err());
        assert!(benchmark_script_resets(Path::new("missing.lua"), 1001).is_err());
    }
}
