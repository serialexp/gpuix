use std::collections::HashSet;
use std::rc::Rc;

use gpui::{prelude::*, px, AnyElement, Context, MouseButton, Pixels, Point, Window};

use crate::{renderer::GpuixView, retained_tree::RetainedTree, text::chrome_text, theme::Theme};

#[derive(Default)]
pub(crate) struct InspectorState {
    pub open: bool,
    pub selected: Option<u64>,
    pub picking: bool,
    menu: Option<(Point<Pixels>, u64)>,
    expanded: HashSet<u64>,
    scroll: gpui::UniformListScrollHandle,
}

impl InspectorState {
    fn select(&mut self, id: u64, tree: &RetainedTree) {
        self.open = true;
        self.selected = Some(id);
        self.menu = None;
        let mut parent = tree.elements.get(&id).and_then(|node| node.parent);
        while let Some(id) = parent {
            self.expanded.insert(id);
            parent = tree.elements.get(&id).and_then(|node| node.parent);
        }
        if tree.root_id == Some(id) {
            self.expanded.insert(id);
        }
        if let Some(index) = self.rows(tree).iter().position(|(row, _)| *row == id) {
            self.scroll
                .scroll_to_item(index, gpui::ScrollStrategy::Center);
        }
    }

    fn rows(&self, tree: &RetainedTree) -> Vec<(u64, usize)> {
        let mut pending = tree.root_id.map(|id| vec![(id, 0)]).unwrap_or_default();
        let mut rows = Vec::new();
        while let Some((id, depth)) = pending.pop() {
            let Some(node) = tree.elements.get(&id) else {
                continue;
            };
            rows.push((id, depth));
            if self.expanded.contains(&id) {
                pending.extend(node.children.iter().rev().map(|id| (*id, depth + 1)));
            }
        }
        rows
    }
}

fn label(value: impl Into<gpui::SharedString>) -> AnyElement {
    chrome_text(value.into(), None)
}

fn action<Event: 'static>(
    cx: &Context<GpuixView>,
    callback: impl Fn(&mut GpuixView, &mut Window, &mut Context<GpuixView>) + 'static,
) -> impl Fn(&Event, &mut Window, &mut gpui::App) + 'static {
    let weak = cx.weak_entity();
    let callback = Rc::new(callback);
    move |_, window, app| {
        app.stop_propagation();
        let weak = weak.clone();
        let callback = callback.clone();
        window.defer(app, move |window, app| {
            let _ = weak.update(app, |view, cx| callback(view, window, cx));
        });
    }
}

fn choose(view: &mut GpuixView, id: u64, window: &mut Window, cx: &mut Context<GpuixView>) {
    view.inspector.select(id, &view.tree.lock().unwrap());
    crate::automation::pin_highlight(window.window_handle().window_id(), Some(id));
    cx.notify();
    let weak = cx.weak_entity();
    window.on_next_frame(move |_, app| {
        let _ = weak.update(app, |_, cx| cx.notify());
    });
}

pub(crate) fn events(state: &InspectorState, cx: &Context<GpuixView>) -> AnyElement {
    let weak = cx.weak_entity();
    let picking = state.picking;
    let dismiss = state.menu.is_some() || picking;
    gpui::canvas(
        |_, _, _| (),
        move |_, _, window, _| {
            let weak_key = weak.clone();
            window.on_root_key_event(move |event: &gpui::KeyDownEvent, phase, window, app| {
                let modifiers = event.keystroke.modifiers;
                let shortcut = event.keystroke.key.eq_ignore_ascii_case("i")
                    && ((cfg!(target_os = "macos") && modifiers.platform && modifiers.alt)
                        || (!cfg!(target_os = "macos") && modifiers.control && modifiers.shift));
                let escape = event.keystroke.key == "escape" && dismiss;
                if phase != gpui::DispatchPhase::Capture || (!shortcut && !escape) {
                    return;
                }
                app.stop_propagation();
                let weak = weak_key.clone();
                window.defer(app, move |window, app| {
                    let _ = weak.update(app, |view, cx| {
                        if escape {
                            view.inspector.menu = None;
                            view.inspector.picking = false;
                        } else if view.inspector.open {
                            view.inspector.open = false;
                            view.inspector.picking = false;
                            view.inspector.menu = None;
                            crate::automation::pin_highlight(
                                window.window_handle().window_id(),
                                None,
                            );
                        } else {
                            let id = view.tree.lock().unwrap().root_id;
                            if let Some(id) = id {
                                choose(view, id, window, cx);
                            }
                        }
                        cx.notify();
                    });
                });
            });
            let weak_down = weak.clone();
            window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, window, app| {
                if phase != gpui::DispatchPhase::Capture {
                    return;
                }
                if event.button != MouseButton::Right
                    && !(picking && event.button == MouseButton::Left)
                {
                    return;
                }
                let point = event.position;
                let Some(id) = crate::automation::painted_candidates(
                    window.window_handle().window_id(),
                    f64::from(point.x),
                    f64::from(point.y),
                )
                .first()
                .copied() else {
                    return;
                };
                app.stop_propagation();
                window.prevent_default();
                let weak = weak_down.clone();
                let right = event.button == MouseButton::Right;
                window.defer(app, move |window, app| {
                    let _ = weak.update(app, |view, cx| {
                        if right {
                            view.inspector.menu = Some((point, id));
                        } else {
                            choose(view, id, window, cx);
                        }
                        cx.notify();
                    });
                });
            });
            window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, window, app| {
                if phase != gpui::DispatchPhase::Capture
                    || !(event.button == MouseButton::Right
                        || (picking && event.button == MouseButton::Left))
                {
                    return;
                }
                if crate::automation::painted_candidates(
                    window.window_handle().window_id(),
                    f64::from(event.position.x),
                    f64::from(event.position.y),
                )
                .is_empty()
                {
                    return;
                }
                app.stop_propagation();
                if event.button == MouseButton::Right {
                    return;
                }
                let weak = weak.clone();
                app.defer(move |app| {
                    let _ = weak.update(app, |view, cx| {
                        view.inspector.picking = false;
                        cx.notify();
                    });
                });
            });
        },
    )
    .absolute()
    .w_0()
    .h_0()
    .into_any_element()
}

pub(crate) fn build(
    state: &mut InspectorState,
    tree: &RetainedTree,
    theme: &Theme,
    window: &mut Window,
    cx: &mut Context<GpuixView>,
) -> AnyElement {
    let metrics = &theme.metrics;
    state.expanded.retain(|id| tree.elements.contains_key(id));
    let selected_mounted = state.selected.is_none_or(|id| {
        let mut current = Some(id);
        while let Some(id) = current {
            if tree.root_id == Some(id) {
                return tree.elements.contains_key(&id);
            }
            current = tree.elements.get(&id).and_then(|node| node.parent);
        }
        false
    });
    if !selected_mounted {
        state.selected = None;
        crate::automation::pin_highlight(window.window_handle().window_id(), None);
    }
    let mut root = gpui::div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .text_size(px(metrics.inspector_text_size))
        .text_color(theme.text);
    if state.open {
        let row_ids = state.rows(tree);
        let row_count = row_ids.len();
        let row_metrics = metrics.clone();
        let render_rows = cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
            let tree = view.tree.lock().unwrap();
            let state = &view.inspector;
            let metrics = &row_metrics;
            let mut rows = Vec::new();
            for &(id, depth) in row_ids.get(range).unwrap_or(&[]) {
                let Some(node) = tree.elements.get(&id) else {
                    continue;
                };
                let disclosure = if node.children.is_empty() {
                    " "
                } else if state.expanded.contains(&id) {
                    "▾"
                } else {
                    "▸"
                };
                let mut row = gpui::div()
                    .id(gpui::SharedString::from(format!(
                        "__gpuix_inspector_row_{id}"
                    )))
                    .flex()
                    .items_center()
                    .h(px(metrics.inspector_row_height))
                    .pl(px(depth as f32 * metrics.inspector_indent))
                    .cursor_pointer()
                    .block_mouse_except_scroll()
                    .when(state.selected == Some(id), |row| {
                        row.bg(gpui::rgb(0x164e63))
                    })
                    .on_click(action(cx, move |view, window, cx| {
                        choose(view, id, window, cx)
                    }));
                row = row.child(
                    gpui::div()
                        .id(gpui::SharedString::from(format!(
                            "__gpuix_inspector_expand_{id}"
                        )))
                        .w(px(metrics.inspector_indent))
                        .flex_shrink_0()
                        .child(label(disclosure))
                        .on_click(action(cx, move |view, _, cx| {
                            if !view.inspector.expanded.remove(&id) {
                                view.inspector.expanded.insert(id);
                            }
                            cx.notify();
                        })),
                );
                let text = format!(
                    "<{}> #{}{}{}",
                    node.element_type,
                    id,
                    node.test_id
                        .as_ref()
                        .map(|value| format!(" [{value}]"))
                        .unwrap_or_default(),
                    node.content
                        .as_ref()
                        .map(|value| format!(" {}", value.chars().take(80).collect::<String>()))
                        .unwrap_or_default()
                );
                rows.push(row.child(label(text)));
            }
            rows
        });
        let rows = gpui::uniform_list("__gpuix_inspector_tree", row_count, render_rows)
            .track_scroll(&state.scroll)
            .flex_1()
            .min_w_0()
            .h_full();
        let properties = state.selected.map(|id| {
            let mut value = tree.node_to_json(id);
            if let Some(object) = value.as_object_mut() {
                object.remove("children");
                let bounds = crate::automation::window_element_bounds(window.window_handle().window_id(), id);
                object.insert("paintedBounds".into(), bounds.map(|bounds| serde_json::json!({ "x": bounds.x, "y": bounds.y, "width": bounds.width, "height": bounds.height })).unwrap_or(serde_json::Value::Null));
            }
            serde_json::to_string_pretty(&value).unwrap_or_default()
        }).unwrap_or_else(|| "Select a host element. Right-click the app, then Inspect element.".into());
        let mut properties_pane = gpui::div()
            .id("__gpuix_inspector_properties")
            .flex_1()
            .min_w_0()
            .overflow_y_scroll()
            .p(px(metrics.inspector_padding));
        for line in properties.lines() {
            properties_pane = properties_pane.child(label(line.to_owned()));
        }
        let header = gpui::div()
            .flex()
            .items_center()
            .h(px(metrics.inspector_row_height + metrics.inspector_padding))
            .px(px(metrics.inspector_padding))
            .gap(px(metrics.inspector_padding))
            .child(label("Inspector · Host elements"))
            .child(gpui::div().flex_1())
            .child(
                gpui::div()
                    .id("__gpuix_inspector_pick")
                    .cursor_pointer()
                    .child(label(if state.picking {
                        "Cancel picking"
                    } else {
                        "Pick element"
                    }))
                    .on_click(action(cx, |view, _, cx| {
                        view.inspector.picking = !view.inspector.picking;
                        cx.notify();
                    })),
            )
            .child(
                gpui::div()
                    .id("__gpuix_inspector_close")
                    .cursor_pointer()
                    .child(label("Close inspector"))
                    .on_click(action(cx, |view, window, cx| {
                        view.inspector.open = false;
                        view.inspector.picking = false;
                        view.inspector.menu = None;
                        crate::automation::pin_highlight(window.window_handle().window_id(), None);
                        cx.notify();
                    })),
            );
        root = root
            .h(px(metrics
                .inspector_height
                .min(f32::from(window.viewport_size().height) / 2.0)))
            .bg(theme.bg)
            .border_t_1()
            .border_color(theme.border)
            .block_mouse_except_scroll()
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .child(header)
            .child(
                gpui::div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(rows)
                    .child(properties_pane),
            );
    }
    if let Some((point, id)) = state.menu {
        let menu = gpui::div()
            .id("__gpuix_inspect_context_menu")
            .p(px(metrics.inspector_padding))
            .bg(theme.bg)
            .border_1()
            .border_color(theme.border)
            .occlude()
            .cursor_pointer()
            .child(label("Inspect element"))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .on_click(action(cx, move |view, window, cx| {
                choose(view, id, window, cx)
            }))
            .on_mouse_down_out(action(cx, |view, _, cx| {
                view.inspector.menu = None;
                cx.notify();
            }));
        root = root.child(
            gpui::deferred(
                gpui::anchored()
                    .position(point)
                    .snap_to_window()
                    .child(menu),
            )
            .priority(usize::MAX - 1),
        );
    }
    root.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn sample_tree() -> RetainedTree {
        let mut tree = RetainedTree::new();
        for id in 1..=4 {
            tree.create_element(id, "div".into());
        }
        tree.set_root(1);
        tree.append_child(1, 2);
        tree.append_child(2, 3);
        tree.set_text(3, "Selected child".into());
        tree
    }

    #[test]
    fn selection_reveals_ancestors_and_excludes_detached_nodes() {
        let tree = sample_tree();
        let mut state = InspectorState::default();
        state.select(3, &tree);
        assert!(state.open);
        assert_eq!(state.selected, Some(3));
        assert_eq!(state.rows(&tree), vec![(1, 0), (2, 1), (3, 2)]);
        state.expanded.remove(&2);
        assert_eq!(state.rows(&tree), vec![(1, 0), (2, 1)]);
        let properties = tree.node_to_json(1);
        assert!(properties.get("children").is_none());
        assert!(tree.subtree_to_json(1, true).get("children").is_some());
    }

    #[test]
    fn context_menu_events_defer_updates_until_the_view_lease_is_released() {
        let cx = gpui::TestAppContext::single();
        let window = cx.update(|app| {
            crate::automation::enable_inspector();
            let mut tree = sample_tree();
            tree.set_style(
                1,
                Arc::new(
                    serde_json::from_value(serde_json::json!({
                        "width": "100%", "height": "100%"
                    }))
                    .unwrap(),
                ),
            );
            let window = crate::renderer::open_main_window(
                &crate::renderer::WindowOptions::default(),
                Arc::new(Mutex::new(tree)),
                None,
                crate::text::SharedSelection::default(),
                app,
            )
            .unwrap();
            gpui::AnyWindowHandle::from(window)
                .update(app, |_, window, app| {
                    let arena = window.draw(app);
                    arena.clear(app);
                })
                .unwrap();
            window
                .update(app, |_, window, cx| {
                    window.dispatch_event(
                        gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                            button: MouseButton::Right,
                            position: gpui::point(px(100.0), px(100.0)),
                            modifiers: Default::default(),
                            click_count: 1,
                            first_mouse: false,
                        }),
                        cx,
                    );
                })
                .unwrap();
            window
        });
        cx.run_until_parked();
        cx.update(|app| {
            assert!(window
                .read_with(app, |view, _| view.inspector.menu.is_some())
                .unwrap());
            assert!(!window
                .read_with(app, |view, _| view.inspector.open)
                .unwrap());
        });
    }

    #[test]
    fn native_dock_resizes_content_without_mutating_the_application_tree() {
        let cx = gpui::TestAppContext::single();
        cx.update(|app| {
            crate::automation::enable_inspector();
            let mut tree = sample_tree();
            tree.set_style(
                1,
                Arc::new(
                    serde_json::from_value(serde_json::json!({"width":"100%", "height":"100%"}))
                        .unwrap(),
                ),
            );
            let revision = tree.elements[&1].subtree_revision;
            let tree = Arc::new(Mutex::new(tree));
            let window = crate::renderer::open_main_window(
                &crate::renderer::WindowOptions::default(),
                tree.clone(),
                None,
                crate::text::SharedSelection::default(),
                app,
            )
            .unwrap();
            let draw = |app: &mut gpui::App| {
                gpui::AnyWindowHandle::from(window)
                    .update(app, |_, window, app| {
                        let arena = window.draw(app);
                        arena.clear(app);
                    })
                    .unwrap();
            };
            draw(app);
            let before = crate::automation::window_bounds(window.window_id())[&1].height;
            window
                .update(app, |view, window, cx| choose(view, 3, window, cx))
                .unwrap();
            draw(app);
            let after = crate::automation::window_bounds(window.window_id())[&1].height;
            assert!(after < before, "dock must reserve space below the app");
            assert_eq!(tree.lock().unwrap().elements[&1].subtree_revision, revision);
            assert_eq!(
                window
                    .read_with(app, |view, _| view.inspector.selected)
                    .unwrap(),
                Some(3)
            );
            window
                .update(app, |view, _, cx| {
                    view.inspector.open = false;
                    cx.notify();
                })
                .unwrap();
            draw(app);
            assert_eq!(
                crate::automation::window_bounds(window.window_id())[&1].height,
                before
            );
        });
    }
}
