//! The [`Host`] a `gpx` JavaScript app has once its window is open: batches
//! go to the window's tree, and the renderer's window methods act on the
//! window as `@gpuix/native`'s napi methods do on macOS, where they also run
//! on the GPUI thread.
//!
//! JS runs in the app's task, never inside a GPUI update (see
//! `quickjs_app.rs`), so a window method can update the window at once and
//! answer straight away. The tree is locked only while a batch applies, so
//! the view methods that read it don't wait on JS.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use futures::channel::mpsc::UnboundedSender;
use serde_json::{json, Value as Json};

use crate::element_tree::HighlightMatch;
use crate::quickjs::{Host, PathPromptResult, WindowCall};
use crate::renderer::{
    debug_frame_overlay_mode_name, element_scroll_offset, list_scroll_top, path_prompt_paths,
    scroll_element_to, scroll_element_to_item, GpuixView,
};
use crate::retained_tree::RetainedTree;
use crate::text::SharedSelection;

pub(crate) struct WindowHost {
    tree: Arc<Mutex<RetainedTree>>,
    window: gpui::WindowHandle<GpuixView>,
    selection: SharedSelection,
    /// Where path prompts send their results, for the app's task to settle.
    prompts: UnboundedSender<(u64, PathPromptResult)>,
    cx: gpui::AsyncApp,
    subscriptions: Rc<RefCell<WindowSubscriptions>>,
}

#[derive(Default)]
pub(crate) struct WindowSubscriptions {
    keys: Option<WindowCall>,
    selection: Option<WindowCall>,
}

impl WindowSubscriptions {
    fn remember(&mut self, call: &WindowCall) -> bool {
        match call {
            WindowCall::SetWindowKeyEvents { .. } => self.keys = Some(call.clone()),
            WindowCall::SetWindowSelectionChange { .. } => self.selection = Some(call.clone()),
            _ => return false,
        }
        true
    }

    pub(crate) fn calls(&self) -> Vec<WindowCall> {
        self.keys
            .iter()
            .chain(self.selection.iter())
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_window_subscriptions_keep_the_latest_registration() {
        let mut subscriptions = WindowSubscriptions::default();
        assert!(subscriptions.remember(&WindowCall::SetWindowKeyEvents {
            key_down: true,
            key_up: false,
            event_id: 1
        }));
        let keys = WindowCall::SetWindowKeyEvents {
            key_down: false,
            key_up: true,
            event_id: 2,
        };
        let selection = WindowCall::SetWindowSelectionChange {
            enabled: true,
            event_id: 3,
        };
        assert!(subscriptions.remember(&keys));
        assert!(subscriptions.remember(&selection));
        assert!(!subscriptions.remember(&WindowCall::FocusNext));
        assert_eq!(subscriptions.calls(), vec![keys, selection]);
    }
}

impl WindowHost {
    /// The host for `window`, which draws `tree` and holds `selection`. Path
    /// prompts' results go to `prompts`.
    pub(crate) fn new(
        tree: Arc<Mutex<RetainedTree>>,
        window: gpui::WindowHandle<GpuixView>,
        selection: SharedSelection,
        prompts: UnboundedSender<(u64, PathPromptResult)>,
        cx: gpui::AsyncApp,
    ) -> Self {
        Self {
            tree,
            window,
            selection,
            prompts,
            cx,
            subscriptions: Rc::default(),
        }
    }

    pub(crate) fn with_subscriptions(
        mut self,
        subscriptions: Rc<RefCell<WindowSubscriptions>>,
    ) -> Self {
        self.subscriptions = subscriptions;
        self
    }
}

impl Host for WindowHost {
    fn with_tree(&mut self, change: &mut dyn FnMut(&mut RetainedTree)) {
        change(&mut self.tree.lock().unwrap());
    }

    fn window(&mut self, call: WindowCall) -> Result<Json, String> {
        let remembered = self.subscriptions.borrow_mut().remember(&call);
        if remembered && self.window.read_with(&self.cx, |_, _| ()).is_err() {
            return Ok(Json::Null);
        }
        let (tree, selection) = (self.tree.clone(), self.selection.clone());
        self.window
            .update(&mut self.cx, move |view, window, cx| {
                run(call, &tree, &selection, view, window, cx)
            })
            .map_err(|error| error.to_string())?
    }

    fn prompt_for_paths(
        &mut self,
        id: u64,
        options: gpui::PathPromptOptions,
    ) -> Result<(), String> {
        let picked = self.cx.update(|cx| cx.prompt_for_paths(options));
        let prompts = self.prompts.clone();
        self.cx
            .foreground_executor()
            .spawn(async move {
                let result = picked
                    .await
                    .map_err(|_| "The path prompt closed without a result".to_string())
                    .and_then(path_prompt_paths);
                // The app reloaded or closed while the picker was open.
                prompts.unbounded_send((id, result)).ok();
            })
            .detach();
        Ok(())
    }
}

/// Runs `call` on the window, as the napi method of the same name does.
fn run(
    call: WindowCall,
    tree: &Mutex<RetainedTree>,
    selection: &SharedSelection,
    view: &mut GpuixView,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<GpuixView>,
) -> Result<Json, String> {
    match call {
        WindowCall::WindowSize => {
            let size = window.viewport_size();
            Ok(json!({
                "width": f64::from(f32::from(size.width)),
                "height": f64::from(f32::from(size.height)),
            }))
        }
        WindowCall::WindowInsets => {
            let insets = window.insets();
            Ok(json!({
                "safeArea": edges(insets.safe_area),
                "ime": edges(insets.ime),
                "effective": edges(insets.effective()),
            }))
        }
        WindowCall::SetDebugFrameOverlay(mode) => {
            window.set_debug_frame_overlay_mode(mode);
            Ok(json!(debug_frame_overlay_mode_name(
                window.debug_frame_overlay_mode()
            )))
        }
        WindowCall::CycleDebugFrameOverlay => {
            window.cycle_debug_frame_overlay_mode();
            Ok(json!(debug_frame_overlay_mode_name(
                window.debug_frame_overlay_mode()
            )))
        }
        WindowCall::DebugFrameOverlay => Ok(json!(debug_frame_overlay_mode_name(
            window.debug_frame_overlay_mode()
        ))),
        WindowCall::ResetDebugFrameOverlayStats => {
            window.reset_debug_frame_overlay_stats();
            Ok(Json::Null)
        }
        WindowCall::DebugFrameOverlayStats => {
            let stats = window.debug_frame_overlay_stats();
            let mut object = serde_json::Map::new();
            // napi leaves out the times it has no samples for.
            for (name, ms) in [
                ("currentMs", stats.current_ms),
                ("p90Ms", stats.p90_ms),
                ("p99Ms", stats.p99_ms),
                ("maxMs", stats.max_ms),
            ] {
                if let Some(ms) = ms {
                    object.insert(name.to_string(), json!(ms as f64));
                }
            }
            object.insert("frames".to_string(), json!(stats.frames as f64));
            object.insert("samples".to_string(), json!(stats.samples as f64));
            Ok(Json::Object(object))
        }
        WindowCall::ActivateWindow => {
            cx.activate(true);
            window.activate_window();
            Ok(Json::Null)
        }
        WindowCall::MinimizeWindow => {
            window.minimize_window();
            Ok(Json::Null)
        }
        WindowCall::ZoomWindow => {
            window.zoom_window();
            Ok(Json::Null)
        }
        WindowCall::ToggleFullscreen => {
            window.toggle_fullscreen();
            Ok(Json::Null)
        }
        WindowCall::SetWindowTitle(title) => {
            view.window_title = title;
            repaint(window, cx);
            Ok(Json::Null)
        }
        WindowCall::FocusElement(id) => {
            view.request_focus(id, window, cx);
            window.refresh();
            Ok(Json::Null)
        }
        WindowCall::FocusNext => {
            window.focus_next(cx);
            Ok(Json::Null)
        }
        WindowCall::FocusPrevious => {
            window.focus_prev(cx);
            Ok(Json::Null)
        }
        WindowCall::FocusedElementId => Ok(json!(view.focused_element_id(window))),
        WindowCall::FocusNextWithin(id) => {
            view.focus_next_within(id, window, cx);
            repaint(window, cx);
            Ok(Json::Null)
        }
        WindowCall::FocusPreviousWithin(id) => {
            view.focus_previous_within(id, window, cx);
            repaint(window, cx);
            Ok(Json::Null)
        }
        WindowCall::SetWindowKeyEvents {
            key_down,
            key_up,
            event_id,
        } => {
            view.window_key_down = key_down;
            view.window_key_up = key_up;
            view.window_key_event_id = event_id;
            repaint(window, cx);
            Ok(Json::Null)
        }
        WindowCall::Blur => {
            window.blur(cx);
            Ok(Json::Null)
        }
        WindowCall::SetWindowSelectionChange { enabled, event_id } => {
            view.set_selection_change_listener(enabled, event_id);
            repaint(window, cx);
            Ok(Json::Null)
        }
        WindowCall::SelectedText => Ok(json!(selection.lock().selected_text())),
        WindowCall::ClearSelection => {
            selection.lock().clear();
            repaint(window, cx);
            Ok(Json::Null)
        }
        WindowCall::ScrollTo { id, x, y } => {
            scroll_element_to(id, x, y);
            repaint(window, cx);
            Ok(Json::Null)
        }
        WindowCall::ScrollToItem { id, index, offset } => {
            scroll_element_to_item(id, index, offset);
            repaint(window, cx);
            Ok(Json::Null)
        }
        WindowCall::ListScrollTop(id) => Ok(json!(list_scroll_top(id))),
        WindowCall::ScrollOffset(id) => Ok(json!(element_scroll_offset(id))),
        WindowCall::ScrollIntoView(id) => {
            if view.scroll_element_into_view(id) {
                repaint(window, cx);
            }
            Ok(Json::Null)
        }
        WindowCall::SetImagePixels {
            id,
            width,
            height,
            bytes,
            format,
        } => view
            .set_image_pixels(id, width, height, bytes, format, window, cx)
            .map(|()| Json::Null),
        WindowCall::SetImage { id, bytes } => view
            .set_encoded_image(id, bytes, window, cx)
            .map(|()| Json::Null),
        WindowCall::RegisterFont(bytes) => {
            view.register_font(bytes, window, cx).map(|()| Json::Null)
        }
        WindowCall::ElementBounds(id) => Ok(crate::automation::get_bounds(id).map_or(
            Json::Null,
            |bounds| {
                json!({
                    "x": bounds.x,
                    "y": bounds.y,
                    "width": bounds.width,
                    "height": bounds.height,
                })
            },
        )),
        WindowCall::ElementsAt { x, y } => {
            let tree = tree.lock().unwrap();
            Ok(json!(tree.elements_at(
                x,
                y,
                &crate::automation::get_bounds
            )))
        }
        WindowCall::PaintedHighlights => {
            let highlights: Vec<HighlightMatch> = crate::text::painted_highlights()
                .into_iter()
                .map(Into::into)
                .collect();
            serde_json::to_value(highlights).map_err(|error| error.to_string())
        }
    }
}

fn repaint(window: &mut gpui::Window, cx: &mut gpui::Context<GpuixView>) {
    cx.notify();
    window.refresh();
}

fn edges(edges: gpui::Edges<gpui::Pixels>) -> Json {
    json!({
        "top": f64::from(f32::from(edges.top)),
        "right": f64::from(f32::from(edges.right)),
        "bottom": f64::from(f32::from(edges.bottom)),
        "left": f64::from(f32::from(edges.left)),
    })
}
