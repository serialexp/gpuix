use gpui::{prelude::*, px, relative, AnyElement, Context, Window};

use crate::inspector::{action, colored_icon, icon, label};
use crate::{
    network::{self, Record},
    renderer::GpuixView,
    theme::Theme,
};

#[derive(Default)]
pub(crate) struct NetworkPanel {
    pub active: bool,
    pub selected: Option<u64>,
    category: usize,
    detail: usize,
    filter: crate::custom_elements::input::ChromeInput,
    scroll: gpui::UniformListScrollHandle,
    refresh: Option<gpui::Task<()>>,
}

struct Row {
    id: u64,
    url: String,
    method: String,
    status: Option<u16>,
    failed: bool,
    kind: &'static str,
    started: f64,
    duration: f64,
    bytes: usize,
    pending: bool,
}

fn failed(record: &Record) -> bool {
    record.status.is_some_and(|status| status >= 400)
        || record.outcome.is_some_and(|outcome| outcome != "Complete")
}

fn status_color(status: Option<u16>, failed: bool, theme: &Theme) -> gpui::Hsla {
    if failed {
        theme.danger
    } else if status.is_none() {
        theme.text_faint
    } else if status.is_some_and(|status| (200..300).contains(&status)) {
        theme.diff_add
    } else {
        theme.text_muted
    }
}

fn size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else {
        format!("{:.1} kB", bytes as f64 / 1024.0)
    }
}

fn cell(text: impl Into<gpui::SharedString>, width: f32, color: gpui::Hsla) -> gpui::Div {
    gpui::div()
        .w(px(width))
        .flex_shrink_0()
        .overflow_hidden()
        .text_ellipsis()
        .text_color(color)
        .child(label(text))
}

fn detail_row(
    name: &str,
    value: impl Into<gpui::SharedString>,
    color: gpui::Hsla,
    theme: &Theme,
) -> AnyElement {
    gpui::div()
        .flex()
        .min_h(px(theme.metrics.inspector_network_detail_row_height))
        .gap(px(theme.metrics.inspector_padding))
        .child(cell(
            name.to_string(),
            theme.metrics.inspector_network_detail_label_width,
            theme.text_faint,
        ))
        .child(
            gpui::div()
                .flex_1()
                .min_w_0()
                .text_color(color)
                .child(label(value)),
        )
        .into_any_element()
}

pub(crate) fn build(
    state: &mut NetworkPanel,
    theme: &Theme,
    window: &mut Window,
    cx: &mut Context<GpuixView>,
) -> AnyElement {
    let Some(log) = network::log(cx) else {
        return label("Network recording is unavailable");
    };
    let query = state.filter.value().to_lowercase();
    let categories = ["All", "Fetch", "JS", "CSS", "Img"];
    let (rows, selected, recording, now, count) = {
        let log = log.0.lock().unwrap();
        let now = log.now();
        let selected = log
            .records
            .iter()
            .find(|record| Some(record.id) == state.selected)
            .cloned();
        let rows = log
            .records
            .iter()
            .filter(|record| {
                (state.category == 0 || record.kind() == categories[state.category])
                    && (query.is_empty()
                        || record.url.to_lowercase().contains(&query)
                        || record.method.to_lowercase().contains(&query)
                        || record
                            .status
                            .is_some_and(|status| status.to_string().contains(&query)))
            })
            .map(|record| Row {
                id: record.id,
                url: record.url.clone(),
                method: record.method.clone(),
                status: record.status,
                failed: failed(record),
                kind: record.kind(),
                started: record.started_ms,
                duration: record.duration(now),
                bytes: record.response_bytes,
                pending: record.ended_ms.is_none(),
            })
            .collect::<Vec<_>>();
        (rows, selected, log.recording, now, log.records.len())
    };
    if selected.is_none() {
        state.selected = None;
    }
    if state.refresh.is_none() && rows.iter().any(|row| row.pending) {
        state.refresh = Some(cx.spawn(async move |view, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(100))
                .await;
            let keep = view
                .update(cx, |view, cx| {
                    let pending = network::log(cx).is_some_and(|log| {
                        log.0
                            .lock()
                            .unwrap()
                            .records
                            .iter()
                            .any(|record| record.ended_ms.is_none())
                    });
                    if view.inspector.open && view.inspector.network.active && pending {
                        cx.notify();
                        true
                    } else {
                        view.inspector.network.refresh = None;
                        false
                    }
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        }));
    }
    let metrics = &theme.metrics;
    let mut toolbar = gpui::div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .h(px(metrics.inspector_network_toolbar_height))
        .px(px(metrics.inspector_padding / 2.0))
        .gap(px(metrics.inspector_padding / 2.0))
        .border_b_1()
        .border_color(theme.border)
        .child(
            gpui::div()
                .id("__gpuix_network_record")
                .aria_label(if recording {
                    "Pause recording"
                } else {
                    "Start recording"
                })
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .size(px(metrics.menu_item_height))
                .rounded(px(metrics.input_radius))
                .hover(|button| button.bg(theme.row_hover))
                .child(colored_icon(
                    "Record",
                    theme,
                    if recording {
                        theme.danger
                    } else {
                        theme.text_faint
                    },
                ))
                .on_click(action(cx, |_, _, cx| {
                    if let Some(log) = network::log(cx) {
                        log.0.lock().unwrap().toggle();
                    }
                    cx.notify();
                })),
        )
        .child(
            gpui::div()
                .id("__gpuix_network_clear")
                .aria_label("Clear requests")
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .size(px(metrics.menu_item_height))
                .rounded(px(metrics.input_radius))
                .hover(|button| button.bg(theme.row_hover))
                .child(icon("Prohibit", theme))
                .on_click(action(cx, |view, _, cx| {
                    if let Some(log) = network::log(cx) {
                        log.0.lock().unwrap().clear();
                    }
                    view.inspector.network.selected = None;
                    cx.notify();
                })),
        )
        .child(
            gpui::div()
                .flex()
                .items_center()
                .min_w_0()
                .w(px(metrics.inspector_network_filter_width))
                .flex_shrink_0()
                .h(px(metrics.menu_item_height))
                .px(px(metrics.inspector_padding / 2.0))
                .rounded(px(metrics.input_radius))
                .border_1()
                .border_color(theme.border_strong)
                .bg(theme.field)
                .child(state.filter.render(theme, window, cx)),
        );
    for (index, category) in categories.into_iter().enumerate() {
        toolbar = toolbar.child(
            gpui::div()
                .id(gpui::SharedString::from(format!(
                    "__gpuix_network_kind_{category}"
                )))
                .cursor_pointer()
                .flex()
                .items_center()
                .h(px(metrics.menu_item_height))
                .px(px(metrics.inspector_padding / 2.0))
                .rounded(px(metrics.input_radius))
                .bg(if state.category == index {
                    theme.row_pressed
                } else {
                    gpui::transparent_black()
                })
                .text_color(if state.category == index {
                    theme.text
                } else {
                    theme.text_muted
                })
                .hover(|button| button.bg(theme.row_hover))
                .child(label(category))
                .on_click(action(cx, move |view, _, cx| {
                    view.inspector.network.category = index;
                    cx.notify();
                })),
        );
    }
    let columns = metrics.inspector_network_columns;
    let gap = metrics.inspector_padding * 0.75;
    let mut headings = gpui::div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .h(px(metrics.inspector_row_height))
        .px(px(metrics.inspector_padding))
        .gap(px(gap))
        .border_b_1()
        .border_color(theme.border);
    for (index, title) in ["Status", "Method", "Name", "Size", "Time", "Waterfall"]
        .into_iter()
        .enumerate()
    {
        headings = if index == 2 {
            headings.child(
                gpui::div()
                    .flex_1()
                    .min_w_0()
                    .text_color(theme.text_faint)
                    .child(label(title)),
            )
        } else {
            let width = columns[if index < 2 { index } else { index - 1 }];
            headings.child(cell(title, width, theme.text_faint))
        };
    }
    let origin = rows.first().map(|row| row.started).unwrap_or(now);
    let span = rows
        .iter()
        .map(|row| row.started + row.duration - origin)
        .fold(1.0_f64, f64::max);
    let selected_id = state.selected;
    let row_count = rows.len();
    let row_theme = theme.clone();
    let render = cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
        range
            .map(|index| {
                let row = &rows[index];
                let id = row.id;
                let theme = &row_theme;
                let selected = selected_id == Some(id);
                let status_color = status_color(row.status, row.failed, theme);
                let status = row
                    .status
                    .map(|status| {
                        if row.pending {
                            format!("{status}…")
                        } else {
                            status.to_string()
                        }
                    })
                    .unwrap_or_else(|| if row.failed { "Failed" } else { "—" }.into());
                let name = row
                    .url
                    .split_once("://")
                    .map(|(_, rest)| rest)
                    .unwrap_or(&row.url)
                    .to_string();
                let icon_name = match row.kind {
                    "Img" => "Image",
                    "JS" => "FileJs",
                    "CSS" => "FileCss",
                    _ => "BracketsCurly",
                };
                gpui::div()
                    .id(gpui::SharedString::from(format!(
                        "__gpuix_network_request_{id}"
                    )))
                    .flex()
                    .items_center()
                    .h(px(theme.metrics.inspector_row_height))
                    .w_full()
                    .px(px(theme.metrics.inspector_padding))
                    .gap(px(gap))
                    .overflow_hidden()
                    .cursor_pointer()
                    .font_family(theme.font_mono.clone())
                    .bg(if selected {
                        theme.row_selected
                    } else {
                        gpui::transparent_black()
                    })
                    .hover(|row| {
                        row.bg(if selected {
                            theme.row_selected
                        } else {
                            theme.row_hover
                        })
                    })
                    .child(cell(status, columns[0], status_color))
                    .child(cell(row.method.clone(), columns[1], theme.text_muted))
                    .child(
                        gpui::div()
                            .flex()
                            .items_center()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .gap(px(theme.metrics.inspector_padding / 2.0))
                            .child(icon(icon_name, theme))
                            .child(
                                gpui::div()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .text_color(if row.failed {
                                        theme.danger
                                    } else if row.pending {
                                        theme.text_faint
                                    } else {
                                        theme.text
                                    })
                                    .child(label(name)),
                            ),
                    )
                    .child(cell(size(row.bytes), columns[2], theme.text_muted))
                    .child(cell(
                        if row.duration >= 1000.0 {
                            format!("{:.1} s", row.duration / 1000.0)
                        } else {
                            format!("{:.0} ms", row.duration)
                        },
                        columns[3],
                        theme.text_muted,
                    ))
                    .child(
                        gpui::div()
                            .relative()
                            .w(px(columns[4]))
                            .flex_shrink_0()
                            .h(px(theme.metrics.scrollbar_width))
                            .child(
                                gpui::div()
                                    .absolute()
                                    .h_full()
                                    .left(relative(((row.started - origin) / span) as f32))
                                    .w(relative((row.duration / span).clamp(0.02, 1.0) as f32))
                                    .rounded(px(theme.metrics.scrollbar_width / 2.0))
                                    .bg(if selected {
                                        theme.accent
                                    } else if row.failed {
                                        theme.danger
                                    } else {
                                        theme.scrollbar_thumb_active
                                    }),
                            ),
                    )
                    .on_click(action(cx, move |view, _, cx| {
                        view.inspector.network.selected = Some(id);
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect::<Vec<AnyElement>>()
    });
    let mut list = gpui::div()
        .flex()
        .flex_col()
        .overflow_hidden()
        .min_w_0()
        .h_full()
        .flex_shrink_0()
        .w(relative(metrics.inspector_network_list_fraction))
        .child(toolbar)
        .child(headings);
    if row_count == 0 {
        list = list.child(
            gpui::div()
                .p(px(metrics.inspector_padding))
                .text_color(theme.text_faint)
                .child(label(if count == 0 {
                    "No requests yet · shared HTTP client only"
                } else {
                    "No matching requests"
                })),
        );
    } else {
        list = list.child(
            gpui::uniform_list("__gpuix_network_requests", row_count, render)
                .track_scroll(&state.scroll)
                .flex_1()
                .min_h_0()
                .w_full(),
        );
    }
    let mut detail_tabs = gpui::div()
        .flex()
        .items_center()
        .overflow_hidden()
        .flex_shrink_0()
        .h(px(metrics.inspector_network_toolbar_height))
        .px(px(metrics.inspector_padding))
        .gap(px(metrics.inspector_padding))
        .border_b_1()
        .border_color(theme.border);
    for (index, title) in ["Headers", "Payload", "Response", "Timing"]
        .into_iter()
        .enumerate()
    {
        detail_tabs = detail_tabs.child(
            gpui::div()
                .id(gpui::SharedString::from(format!(
                    "__gpuix_network_detail_{title}"
                )))
                .cursor_pointer()
                .h_full()
                .flex()
                .items_center()
                .border_b(px(metrics.control_ring_width))
                .border_color(if state.detail == index {
                    theme.accent
                } else {
                    gpui::transparent_black()
                })
                .text_color(if state.detail == index {
                    theme.text
                } else {
                    theme.text_muted
                })
                .child(label(title))
                .on_click(action(cx, move |view, _, cx| {
                    view.inspector.network.detail = index;
                    cx.notify();
                })),
        );
    }
    let mut details = gpui::div()
        .id("__gpuix_network_detail_body")
        .flex_1()
        .min_h_0()
        .min_w_0()
        .overflow_y_scroll()
        .p(px(metrics.inspector_padding))
        .font_family(theme.font_mono.clone());
    if let Some(record) = selected {
        match state.detail {
            0 => {
                details = details
                    .child(detail_row("URL", record.url.clone(), theme.text, theme))
                    .child(detail_row(
                        "Method",
                        record.method.clone(),
                        theme.text,
                        theme,
                    ))
                    .child(detail_row(
                        "Status",
                        record
                            .status
                            .map(|status| status.to_string())
                            .unwrap_or_else(|| {
                                if failed(&record) { "Failed" } else { "Pending" }.into()
                            }),
                        status_color(record.status, failed(&record), theme),
                        theme,
                    ))
                    .child(detail_row(
                        "Outcome",
                        record.outcome.unwrap_or("Pending"),
                        theme.text,
                        theme,
                    ))
                    .child(detail_row(
                        "Duration",
                        format!("{:.1} ms", record.duration(now)),
                        theme.text,
                        theme,
                    ));
                for (title, headers) in [
                    ("Request headers", &record.request_headers),
                    ("Response headers", &record.response_headers),
                ] {
                    details = details.child(
                        gpui::div()
                            .mt(px(metrics.inspector_padding))
                            .text_color(theme.text_muted)
                            .child(label(title)),
                    );
                    for (name, value) in headers {
                        details = details.child(detail_row(
                            name,
                            value.clone(),
                            theme.syntax.string,
                            theme,
                        ));
                    }
                }
            }
            1 | 2 => {
                for line in record.preview(state.detail == 2).lines() {
                    details = details.child(
                        gpui::div()
                            .min_h(px(metrics.inspector_network_detail_row_height))
                            .text_color(theme.syntax.string)
                            .child(label(line.to_string())),
                    );
                }
            }
            _ => {
                details = details
                    .child(detail_row(
                        "Started",
                        format!("{:.1} ms", record.started_ms),
                        theme.text,
                        theme,
                    ))
                    .child(detail_row(
                        "Headers",
                        record
                            .headers_ms
                            .map(|time| format!("{:.1} ms", time - record.started_ms))
                            .unwrap_or_else(|| "Pending".into()),
                        theme.text,
                        theme,
                    ))
                    .child(detail_row(
                        "Body",
                        record
                            .headers_ms
                            .map(|time| format!("{:.1} ms", record.ended_ms.unwrap_or(now) - time))
                            .unwrap_or_else(|| "Pending".into()),
                        theme.text,
                        theme,
                    ))
                    .child(detail_row(
                        "Total",
                        format!("{:.1} ms", record.duration(now)),
                        theme.text,
                        theme,
                    ))
                    .child(detail_row(
                        "Sent",
                        size(record.request_bytes),
                        theme.text,
                        theme,
                    ))
                    .child(detail_row(
                        "Received",
                        size(record.response_bytes),
                        theme.text,
                        theme,
                    ))
                    .child(detail_row(
                        "Phases",
                        "DNS/TLS/connection timings unavailable",
                        theme.text_faint,
                        theme,
                    ));
            }
        }
    } else {
        details = details
            .text_color(theme.text_faint)
            .child(label("Select a request to inspect it"));
    }
    gpui::div()
        .flex()
        .flex_1()
        .min_h_0()
        .w_full()
        .text_size(px(metrics.inspector_network_text_size))
        .child(list)
        .child(
            gpui::div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .h_full()
                .border_l_1()
                .border_color(theme.border)
                .child(detail_tabs)
                .child(details),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::AsyncReadExt;
    use gpui::http_client::{AsyncBody, Request, Response};
    use std::sync::{Arc, Mutex};

    #[test]
    fn native_network_details_filter_and_chrome_isolation() {
        let mut cx = gpui::TestAppContext::single();
        let (window, dock_top) = cx.update(|app| {
            crate::automation::enable_inspector();
            crate::custom_elements::input::init(app);
            app.set_http_client(gpui::http_client::FakeHttpClient::create(|_| async {
                Ok(Response::builder()
                    .status(422)
                    .header("content-type", "application/json")
                    .body(AsyncBody::from(
                        r#"{"message":"validation failed","token":"secret-response"}"#,
                    ))
                    .unwrap())
            }));
            network::install(app);
            let mut response = futures::executor::block_on(
                app.http_client().send(
                    Request::builder()
                        .uri("http://example.test/request?token=secret-query")
                        .header("authorization", "secret-header")
                        .body(AsyncBody::empty())
                        .unwrap(),
                ),
            )
            .unwrap();
            futures::executor::block_on(response.body_mut().read_to_end(&mut Vec::new())).unwrap();
            let mut tree = crate::retained_tree::RetainedTree::new();
            tree.create_element(1, "div".into());
            tree.set_root(1);
            tree.set_style(
                1,
                Arc::new(
                    serde_json::from_value(serde_json::json!({"width":"100%","height":"100%"}))
                        .unwrap(),
                ),
            );
            let tree = Arc::new(Mutex::new(tree));
            let window = crate::renderer::open_main_window(
                &crate::renderer::WindowOptions {
                    width: Some(1000.0),
                    height: Some(800.0),
                    focus: Some(false),
                    ..Default::default()
                },
                tree.clone(),
                None,
                Default::default(),
                app,
            )
            .unwrap();
            window
                .update(app, |view, _, cx| {
                    view.inspector.open = true;
                    view.inspector.network.active = true;
                    view.inspector.network.selected = Some(1);
                    cx.notify();
                })
                .unwrap();
            draw(window, app);
            let painted = crate::text::painted_text().join("\n");
            assert!(painted.contains("Network"));
            assert!(painted.contains("422"));
            assert!(painted.contains("secret-query"));
            assert!(painted.contains("secret-header"));
            window
                .update(app, |view, _, cx| {
                    view.inspector.network.detail = 2;
                    cx.notify();
                })
                .unwrap();
            draw(window, app);
            let painted = crate::text::painted_text().join("\n");
            assert!(painted.contains("validation failed"));
            assert!(painted.contains("secret-response"));
            assert_eq!(tree.lock().unwrap().elements.len(), 1);
            let dock_top = crate::automation::window_bounds(window.window_id())[&1].height as f32;
            assert!(crate::automation::painted_candidates(
                window.window_id(),
                100.0,
                f64::from(dock_top + 44.0)
            )
            .is_empty());
            window
                .update(app, |_, window, cx| {
                    window.dispatch_event(
                        gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                            button: gpui::MouseButton::Left,
                            position: gpui::point(px(100.0), px(dock_top + 44.0)),
                            modifiers: Default::default(),
                            click_count: 1,
                            first_mouse: false,
                        }),
                        cx,
                    );
                })
                .unwrap();
            (window, dock_top)
        });
        cx.simulate_input(window.into(), "nothing-matches");
        cx.run_until_parked();
        cx.update(|app| {
            draw(window, app);
            assert!(
                crate::text::painted_text()
                    .join("\n")
                    .contains("No matching requests"),
                "filter at {dock_top} must accept native input"
            );
        });
    }

    fn draw(window: gpui::WindowHandle<GpuixView>, app: &mut gpui::App) {
        gpui::AnyWindowHandle::from(window)
            .update(app, |_, window, app| {
                let arena = window.draw(app);
                arena.clear(app);
            })
            .unwrap();
    }
}
