use std::collections::HashSet;
use std::rc::Rc;

use gpui::{prelude::*, px, AnyElement, Context, MouseButton, Pixels, Point, Window};

use crate::{renderer::GpuixView, retained_tree::RetainedTree, text::chrome_text, theme::Theme};

#[derive(Default)]
pub(crate) struct InspectorState {
    #[cfg(not(target_family = "wasm"))]
    pub network: crate::inspector_network::NetworkPanel,
    pub open: bool,
    pub selected: Option<u64>,
    pub picking: bool,
    menu: Option<(Point<Pixels>, u64)>,
    expanded: HashSet<u64>,
    scroll: gpui::UniformListScrollHandle,
}

impl InspectorState {
    fn select(&mut self, id: u64, tree: &RetainedTree) {
        #[cfg(not(target_family = "wasm"))]
        {
            self.network.active = false;
        }
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

pub(crate) fn label(value: impl Into<gpui::SharedString>) -> AnyElement {
    chrome_text(value.into(), None)
}

pub(crate) fn icon(name: &str, theme: &Theme) -> AnyElement {
    colored_icon(name, theme, theme.text_muted)
}

pub(crate) fn colored_icon(name: &str, theme: &Theme, color: gpui::Hsla) -> AnyElement {
    #[cfg(any(feature = "lua54", feature = "luajit"))]
    if let Ok(token) = crate::phosphor::token(name, "regular") {
        if let Some(bytes) = crate::phosphor::source_from_token(&token) {
            return gpui::svg()
                .data(bytes)
                .id(gpui::SharedString::from(format!(
                    "__gpuix_inspector_icon_{name}"
                )))
                .size(px(theme.metrics.inspector_icon_size))
                .flex_shrink_0()
                .text_color(color)
                .into_any_element();
        }
    }
    gpui::div()
        .size(px(theme.metrics.inspector_icon_size))
        .into_any_element()
}

fn property_section(name: &str, value: &serde_json::Value, theme: &Theme) -> AnyElement {
    let metrics = &theme.metrics;
    let mut section = gpui::div()
        .flex()
        .flex_col()
        .gap(px(metrics.inspector_padding / 2.0))
        .pb(px(metrics.inspector_padding))
        .mb(px(metrics.inspector_padding))
        .border_b_1()
        .border_color(theme.border)
        .child(
            gpui::div()
                .text_color(theme.text_muted)
                .child(label(name.to_owned())),
        );
    let values: Vec<(String, serde_json::Value)> = match value {
        serde_json::Value::Object(values) => values
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        serde_json::Value::Null => Vec::new(),
        value => vec![(String::new(), value.clone())],
    };
    if values.is_empty() {
        section = section.child(
            gpui::div()
                .text_color(theme.text_faint)
                .child(label("None")),
        );
    }
    for (key, value) in values {
        let color = if value.is_number() || value.is_boolean() {
            theme.syntax.number
        } else if value.is_string() {
            theme.syntax.string
        } else {
            theme.text_muted
        };
        let display = if let Some(text) = value.as_str() {
            text.to_owned()
        } else {
            value.to_string()
        };
        section = section.child(
            gpui::div()
                .flex()
                .gap(px(metrics.inspector_padding))
                .font_family(theme.font_mono.clone())
                .child(
                    gpui::div()
                        .flex_shrink_0()
                        .text_color(theme.syntax.keyword)
                        .child(label(key)),
                )
                .child(
                    gpui::div()
                        .min_w_0()
                        .text_color(color)
                        .child(label(display)),
                ),
        );
    }
    section.into_any_element()
}

pub(crate) fn action<Event: 'static>(
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
        #[cfg(not(target_family = "wasm"))]
        crate::network::install(cx);
        #[cfg(not(target_family = "wasm"))]
        let network_active = state.network.active;
        #[cfg(target_family = "wasm")]
        let network_active = false;
        #[cfg(not(target_family = "wasm"))]
        let network_content = network_active
            .then(|| crate::inspector_network::build(&mut state.network, theme, window, cx));
        #[cfg(target_family = "wasm")]
        let network_content: Option<AnyElement> = None;
        let row_ids = if network_active {
            Vec::new()
        } else {
            state.rows(tree)
        };
        let row_count = row_ids.len();
        let row_metrics = metrics.clone();
        let row_theme = theme.clone();
        let render_rows = cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
            let tree = view.tree.lock().unwrap();
            let state = &view.inspector;
            let metrics = &row_metrics;
            let theme = &row_theme;
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
                    .w_full()
                    .overflow_hidden()
                    .gap(px(metrics.inspector_padding / 2.0))
                    .h(px(metrics.inspector_row_height))
                    .pl(px(
                        metrics.inspector_padding + depth as f32 * metrics.inspector_indent
                    ))
                    .cursor_pointer()
                    .block_mouse_except_scroll()
                    .hover(|row| row.bg(theme.row_hover))
                    .when(state.selected == Some(id), |row| row.bg(theme.row_selected))
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
                        .text_color(if state.expanded.contains(&id) {
                            theme.accent
                        } else {
                            theme.text_faint
                        })
                        .child(label(disclosure))
                        .on_click(action(cx, move |view, _, cx| {
                            if !view.inspector.expanded.remove(&id) {
                                view.inspector.expanded.insert(id);
                            }
                            cx.notify();
                        })),
                );
                row = row
                    .child(icon(
                        match node.element_type.as_str() {
                            "text" => "TextT",
                            "img" => "Image",
                            "svg" => "Shapes",
                            _ => "Square",
                        },
                        &row_theme,
                    ))
                    .child(
                        gpui::div()
                            .text_color(row_theme.syntax.tag)
                            .child(label(format!("<{}>", node.element_type))),
                    )
                    .child(
                        gpui::div()
                            .text_color(row_theme.text_faint)
                            .child(label(format!("#{id}"))),
                    );
                if let Some(test_id) = &node.test_id {
                    row = row.child(
                        gpui::div()
                            .text_color(row_theme.syntax.attribute)
                            .child(label(format!("[{test_id}]"))),
                    );
                }
                if let Some(content) = &node.content {
                    row = row.child(
                        gpui::div()
                            .text_color(row_theme.text_muted)
                            .child(label(content.chars().take(80).collect::<String>())),
                    );
                }
                rows.push(row);
            }
            rows
        });
        let rows = gpui::uniform_list("__gpuix_inspector_tree", row_count, render_rows)
            .track_scroll(&state.scroll)
            .flex_1()
            .min_w_0()
            .h_full();
        let mut properties_pane = gpui::div()
            .id("__gpuix_inspector_properties")
            .flex_1()
            .min_w_0()
            .overflow_y_scroll()
            .border_l_1()
            .border_color(theme.border)
            .p(px(metrics.inspector_padding));
        if let Some(id) = state.selected {
            let mut value = tree.node_to_json(id);
            let bounds =
                crate::automation::window_element_bounds(window.window_handle().window_id(), id);
            value["paintedBounds"] = bounds
                .map(|bounds| {
                    serde_json::json!({
                        "x": bounds.x, "y": bounds.y, "width": bounds.width, "height": bounds.height
                    })
                })
                .unwrap_or(serde_json::Value::Null);
            let mut identity = serde_json::Map::new();
            for key in ["id", "type", "testId", "text"] {
                if let Some(value) = value.get(key) {
                    identity.insert(key.to_owned(), value.clone());
                }
            }
            properties_pane = properties_pane
                .child(property_section(
                    "Element",
                    &serde_json::Value::Object(identity),
                    theme,
                ))
                .child(property_section(
                    "Layout · last paint",
                    &value["paintedBounds"],
                    theme,
                ))
                .child(property_section("Styles", &value["style"], theme))
                .child(property_section("Properties", &value["customProps"], theme))
                .child(property_section("Events", &value["events"], theme));
        } else {
            properties_pane =
                properties_pane.child(gpui::div().text_color(theme.text_muted).child(label(
                    "Select an element in the tree, or use the picker to inspect the application.",
                )));
        }
        let header = gpui::div()
            .flex()
            .items_center()
            .h(px(metrics.inspector_row_height + metrics.inspector_padding))
            .px(px(metrics.inspector_padding / 2.0))
            .gap(px(metrics.inspector_padding / 4.0))
            .border_b_1()
            .border_color(theme.border)
            .child(
                gpui::div()
                    .id("__gpuix_inspector_pick")
                    .aria_label(if state.picking {
                        "Cancel picking"
                    } else {
                        "Pick element"
                    })
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(metrics.menu_item_height))
                    .rounded(px(metrics.inspector_padding / 2.0))
                    .hover(|button| button.bg(theme.row_hover))
                    .when(state.picking, |button| {
                        button.bg(theme.row_selected).text_color(theme.accent)
                    })
                    .child(colored_icon(
                        "CursorClick",
                        theme,
                        if state.picking {
                            theme.accent
                        } else {
                            theme.text_muted
                        },
                    ))
                    .on_click(action(cx, |view, _, cx| {
                        view.inspector.picking = !view.inspector.picking;
                        cx.notify();
                    })),
            )
            .child(
                gpui::div()
                    .w(px(1.0))
                    .h(px(metrics.inspector_icon_size))
                    .mx(px(metrics.inspector_padding / 2.0))
                    .bg(theme.border_strong),
            )
            .child(
                gpui::div()
                    .id("__gpuix_inspector_elements_tab")
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(metrics.inspector_padding * 0.75))
                    .px(px(metrics.inspector_padding))
                    .h(px(metrics.inspector_row_height))
                    .rounded(px(metrics.inspector_padding / 2.0))
                    .bg(if network_active {
                        gpui::transparent_black()
                    } else {
                        theme.row_hover
                    })
                    .child(icon("TreeStructure", theme))
                    .child(label("Elements"))
                    .on_click(action(cx, |view, window, cx| {
                        #[cfg(not(target_family = "wasm"))]
                        {
                            view.inspector.network.active = false;
                        }
                        crate::automation::pin_highlight(
                            window.window_handle().window_id(),
                            view.inspector.selected,
                        );
                        cx.notify();
                    })),
            )
            .when(cfg!(not(target_family = "wasm")), |header| {
                header.child(
                    gpui::div()
                        .id("__gpuix_inspector_network_tab")
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap(px(metrics.inspector_padding * 0.75))
                        .px(px(metrics.inspector_padding))
                        .h(px(metrics.inspector_row_height))
                        .rounded(px(metrics.inspector_padding / 2.0))
                        .bg(if network_active {
                            theme.row_hover
                        } else {
                            gpui::transparent_black()
                        })
                        .child(icon("WifiHigh", theme))
                        .child(label("Network"))
                        .on_click(action(cx, |view, window, cx| {
                            #[cfg(not(target_family = "wasm"))]
                            {
                                view.inspector.network.active = true;
                            }
                            view.inspector.picking = false;
                            crate::automation::pin_highlight(
                                window.window_handle().window_id(),
                                None,
                            );
                            cx.notify();
                        })),
                )
            })
            .child(gpui::div().flex_1())
            .child(
                gpui::div()
                    .id("__gpuix_inspector_close")
                    .aria_label("Close inspector")
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(metrics.menu_item_height))
                    .rounded(px(metrics.inspector_padding / 2.0))
                    .hover(|button| button.bg(theme.row_hover))
                    .child(icon("X", theme))
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
            .bg(theme.surface)
            .border_t_1()
            .border_color(theme.border)
            .block_mouse_except_scroll()
            .on_painted(|bounds, window, _| {
                crate::automation::exclude_inspector_bounds(bounds, window);
            })
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .child(header)
            .child(network_content.unwrap_or_else(|| {
                gpui::div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(rows)
                    .child(properties_pane)
                    .into_any_element()
            }));
    }
    if let Some((point, id)) = state.menu {
        let menu = gpui::div()
            .id("__gpuix_inspect_context_menu")
            .p(px(metrics.inspector_padding))
            .flex()
            .items_center()
            .gap(px(metrics.inspector_padding))
            .rounded(px(metrics.menu_radius))
            .bg(theme.raised)
            .shadow(vec![gpui::BoxShadow {
                color: theme.shadow,
                offset: gpui::point(px(0.0), px(metrics.inspector_padding / 2.0)),
                blur_radius: px(metrics.inspector_padding * 2.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .border_1()
            .border_color(theme.border_strong)
            .occlude()
            .on_painted(|bounds, window, _| {
                crate::automation::exclude_inspector_bounds(bounds, window);
            })
            .cursor_pointer()
            .hover(|menu| menu.bg(theme.raised.blend(theme.row_hover)))
            .child(icon("CrosshairSimple", theme))
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
            assert!(
                crate::automation::painted_candidates(window.window_id(), 12.0, after + 16.0,)
                    .is_empty(),
                "inspector chrome must not pick application nodes behind it"
            );
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
