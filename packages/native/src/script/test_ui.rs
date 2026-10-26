use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, PathBuf};

use gpui::{prelude::*, App, Global, WindowHandle};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Case {
    index: usize,
    name: String,
    suite: Option<String>,
    status: Status,
    #[serde(default)]
    duration_ms: u64,
    error: Option<String>,
    report_path: Option<PathBuf>,
    #[serde(skip)]
    steps: std::sync::Arc<Vec<EvidenceStep>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvidenceStep {
    step: serde_json::Value,
    status: Status,
    duration_ms: u64,
    error: Option<String>,
    screenshot: Option<PathBuf>,
    tree: Option<PathBuf>,
    snapshot: Option<serde_json::Value>,
    #[serde(default)]
    artifact_errors: Vec<String>,
}

fn evidence(path: &std::path::Path) -> Result<Vec<EvidenceStep>, String> {
    if !path.is_absolute() {
        return Err("case report path must be absolute".into());
    }
    #[derive(Deserialize)]
    struct Report {
        steps: Vec<EvidenceStep>,
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|error| error.to_string())?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("case report exceeds 16 MiB".into());
    }
    let mut report: Report = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if report.steps.len() > 10_000 {
        return Err("case report exceeds 10000 steps".into());
    }
    for step in &mut report.steps {
        for screenshot in [&mut step.screenshot, &mut step.tree].into_iter().flatten() {
            if screenshot.components().count() != 1
                || !matches!(screenshot.components().next(), Some(Component::Normal(_)))
            {
                return Err("screenshot must be a filename within the case directory".into());
            }
            *screenshot = path
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join(&screenshot);
        }
    }
    Ok(report.steps)
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Pending,
    Running,
    Passed,
    Failed,
    Skipped,
}

struct Results {
    name: String,
    cases: BTreeMap<usize, Case>,
    selected_case: Option<usize>,
    selected_step: Option<usize>,
    selection: crate::text::SharedSelection,
    focus: gpui::FocusHandle,
    filter: crate::custom_elements::input::ChromeInput,
    tree_mode: bool,
    commands: Vec<String>,
    case_scroll: gpui::UniformListScrollHandle,
    step_scroll: gpui::UniformListScrollHandle,
    tree_cache: Option<(PathBuf, String)>,
}

struct ResultsWindow(WindowHandle<Results>);
impl Global for ResultsWindow {}

struct Presentation {
    background: bool,
}
impl Global for Presentation {}

pub fn run_test_results(background: bool) -> Result<(), String> {
    let server = super::server::Server::start(Some(0))?.ok_or("results server unavailable")?;
    gpui_platform::application()
        .with_quit_mode(gpui::QuitMode::Explicit)
        .run(move |cx| {
            crate::renderer::init_app("GPUIX Test Results", cx);
            cx.set_global(Presentation { background });
            server.attach(cx);
        });
    Ok(())
}

pub(crate) fn commands(cx: &mut App) -> Result<serde_json::Value, String> {
    let handle = cx
        .try_global::<ResultsWindow>()
        .ok_or("results window is not open")?
        .0;
    handle
        .update(cx, |view, _, _| {
            serde_json::json!(std::mem::take(&mut view.commands))
        })
        .map_err(|error| error.to_string())
}

pub(crate) fn request_run(failed_only: bool, cx: &mut App) -> Result<serde_json::Value, String> {
    let handle = cx
        .try_global::<ResultsWindow>()
        .ok_or("results window is not open")?
        .0;
    handle
        .update(cx, |view, _, cx| {
            if view
                .cases
                .values()
                .any(|case| matches!(case.status, Status::Pending | Status::Running))
                || !view.commands.is_empty()
            {
                return Err("a test run is already pending or running".into());
            }
            if failed_only
                && !view
                    .cases
                    .values()
                    .any(|case| matches!(case.status, Status::Failed))
            {
                return Err("there are no failed tests".into());
            }
            view.commands
                .push(if failed_only { "failed" } else { "all" }.into());
            cx.notify();
            Ok(serde_json::json!({"queued":true}))
        })
        .map_err(|error| error.to_string())?
}

pub(crate) fn update(
    name: String,
    mut cases: Vec<Case>,
    cx: &mut App,
) -> Result<serde_json::Value, String> {
    if name.len() > 1024
        || cases.len() > 32
        || cases.iter().any(|case| {
            case.index >= 10_000
                || case.name.len() > 1024
                || case.error.as_ref().is_some_and(|error| error.len() > 4096)
        })
    {
        return Err("test results update exceeds limits".into());
    }
    for case in &mut cases {
        if let Some(path) = &case.report_path {
            case.steps = std::sync::Arc::new(evidence(path)?);
        }
    }
    let handle = if let Some(handle) = cx.try_global::<ResultsWindow>() {
        handle.0
    } else {
        let metrics = crate::theme::Theme::default().metrics;
        let foreground = !cx
            .try_global::<Presentation>()
            .is_some_and(|presentation| presentation.background);
        let handle = cx
            .open_window(
                gpui::WindowOptions {
                    focus: foreground,
                    window_bounds: Some(gpui::WindowBounds::centered(
                        gpui::size(
                            gpui::px(metrics.test_window_width),
                            gpui::px(metrics.test_window_height),
                        ),
                        cx,
                    )),
                    ..Default::default()
                },
                |window, cx| {
                    window.set_window_title("GPUIX Test Results");
                    cx.new(|cx| Results {
                        name: name.clone(),
                        cases: BTreeMap::new(),
                        selected_case: None,
                        selected_step: None,
                        selection: Default::default(),
                        focus: cx.focus_handle(),
                        filter: crate::custom_elements::input::ChromeInput::new("Filter tests"),
                        tree_mode: false,
                        commands: Vec::new(),
                        case_scroll: Default::default(),
                        step_scroll: Default::default(),
                        tree_cache: None,
                    })
                },
            )
            .map_err(|error| error.to_string())?;
        cx.set_global(ResultsWindow(handle));
        if foreground {
            cx.activate(true);
        }
        let id = handle.window_id();
        cx.on_window_closed(move |cx, closed| {
            if closed == id {
                cx.quit();
            }
        })
        .detach();
        handle
    };
    handle
        .update(cx, |view, window, cx| {
            view.name = name;
            for case in cases {
                if view.selected_case == Some(case.index) {
                    view.tree_cache = None;
                }
                if view.selected_case.is_none() {
                    view.selected_case = Some(case.index);
                }
                view.cases.insert(case.index, case);
            }
            if !view.focus.is_focused(window) && window.focused(cx).is_none() {
                view.focus.focus(window, cx);
            }
            cx.notify();
        })
        .map_err(|error| error.to_string())?;
    Ok(serde_json::json!({"window": handle.window_id().as_u64()}))
}

enum TestRow {
    Group(String),
    Test(Case),
}

fn status_icon(status: Status, theme: &crate::theme::Theme) -> gpui::AnyElement {
    let name = match status {
        Status::Passed => "CheckCircle",
        Status::Failed => "XCircle",
        Status::Skipped => "CircleDashed",
        Status::Running => "Spinner",
        Status::Pending => "Circle",
    };
    crate::inspector::colored_icon(name, theme, status.appearance(theme).1)
}

fn section(title: impl Into<gpui::SharedString>, theme: &crate::theme::Theme) -> gpui::Div {
    gpui::div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .h(gpui::px(theme.metrics.test_section_height))
        .px(gpui::px(theme.metrics.test_padding))
        .gap(gpui::px(theme.metrics.control_gap))
        .border_b_1()
        .border_color(theme.border)
        .child(
            gpui::div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(crate::inspector::label(title)),
        )
}

fn short(value: &str) -> String {
    let mut text = value.chars().take(4096).collect::<String>();
    if text.len() < value.len() {
        text.push_str("…");
    }
    text
}

impl Results {
    fn visible_cases(&self) -> Vec<usize> {
        let filter = self.filter.value().to_lowercase();
        self.cases
            .values()
            .filter(|case| {
                filter.is_empty()
                    || case.name.to_lowercase().contains(&filter)
                    || case
                        .suite
                        .as_deref()
                        .unwrap_or_default()
                        .to_lowercase()
                        .contains(&filter)
            })
            .map(|case| case.index)
            .collect()
    }

    fn active_step(&self) -> Option<usize> {
        let steps = &self.cases.get(&self.selected_case?)?.steps;
        self.selected_step
            .filter(|index| *index < steps.len())
            .or_else(|| {
                steps
                    .iter()
                    .position(|step| matches!(step.status, Status::Failed))
            })
            .or_else(|| (!steps.is_empty()).then_some(0))
    }

    fn navigate(&mut self, key: &str) {
        let cases = self.visible_cases();
        if matches!(key, "left" | "right") && !cases.is_empty() {
            let index = cases
                .iter()
                .position(|id| Some(*id) == self.selected_case)
                .unwrap_or_default();
            let next = if key == "left" {
                index.saturating_sub(1)
            } else {
                (index + 1).min(cases.len() - 1)
            };
            self.selected_case = Some(cases[next]);
            self.selected_step = None;
            self.tree_cache = None;
            let mut group = None;
            let mut row = 0;
            for id in &cases {
                let suite = &self.cases[id].suite;
                if group.as_ref() != Some(suite) {
                    if suite.is_some() {
                        row += 1;
                    }
                    group = Some(suite.clone());
                }
                if Some(*id) == self.selected_case {
                    self.case_scroll
                        .scroll_to_item(row, gpui::ScrollStrategy::Nearest);
                    break;
                }
                row += 1;
            }
        } else if matches!(key, "up" | "down") {
            let count = self
                .selected_case
                .and_then(|id| self.cases.get(&id))
                .map(|case| case.steps.len())
                .unwrap_or_default();
            if count > 0 {
                let index = self.active_step().unwrap_or_default();
                let next = if key == "up" {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(count - 1)
                };
                self.selected_step = Some(next);
                self.step_scroll
                    .scroll_to_item(next, gpui::ScrollStrategy::Nearest);
                self.tree_cache = None;
            }
        }
    }

    fn details(
        &mut self,
        theme: &crate::theme::Theme,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::AnyElement {
        use crate::inspector::{icon, label};
        let metrics = theme.metrics;
        let selected_step = self.active_step();
        let case = self.selected_case.and_then(|index| self.cases.get(&index));
        let count = case.map(|case| case.steps.len()).unwrap_or_default();
        let detail = case
            .and_then(|case| case.steps.get(selected_step?))
            .cloned();
        let Some(step) = detail else {
            return gpui::div()
                .flex_1()
                .min_w_0()
                .p(gpui::px(metrics.test_padding))
                .text_color(theme.text_muted)
                .child(label("Select a test and step to review its evidence"))
                .into_any_element();
        };
        let action = step.step["action"].as_str().unwrap_or("Step");
        let (status, color) = step.status.appearance(theme);
        let header = section(action.to_string(), theme)
            .font_family(theme.font_mono.clone())
            .child(
                gpui::div()
                    .rounded(gpui::px(metrics.input_radius))
                    .px(gpui::px(metrics.control_gap))
                    .bg(if matches!(step.status, Status::Failed) {
                        theme.danger_wash
                    } else {
                        theme.row_selected
                    })
                    .text_color(color)
                    .child(label(status)),
            )
            .child(gpui::div().flex_1())
            .child(
                gpui::div()
                    .text_color(theme.text_faint)
                    .child(label(format!(
                        "step {} of {count} · {} ms",
                        selected_step.unwrap_or_default() + 1,
                        step.duration_ms
                    ))),
            );
        let mut args = gpui::div()
            .flex()
            .flex_col()
            .gap(gpui::px(metrics.control_gap))
            .flex_shrink_0();
        if let Some(values) = step.step.as_object() {
            for (key, value) in values
                .iter()
                .filter(|(key, _)| !matches!(key.as_str(), "action" | "name" | "timeoutMs"))
            {
                let color = match value {
                    serde_json::Value::String(_) => theme.syntax.string,
                    serde_json::Value::Number(_) => theme.syntax.number,
                    serde_json::Value::Bool(_) | serde_json::Value::Null => theme.syntax.keyword,
                    _ => theme.text,
                };
                args = args.child(
                    gpui::div()
                        .flex()
                        .gap(gpui::px(metrics.control_gap))
                        .font_family(theme.font_mono.clone())
                        .child(
                            gpui::div()
                                .w(gpui::px(metrics.test_argument_width))
                                .flex_shrink_0()
                                .text_color(theme.text_faint)
                                .child(label(key.clone())),
                        )
                        .child(
                            gpui::div()
                                .flex_1()
                                .min_w_0()
                                .text_color(color)
                                .child(label(short(&value.to_string()))),
                        ),
                );
            }
        }
        let mut content = gpui::div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .p(gpui::px(metrics.test_padding))
            .gap(gpui::px(metrics.test_padding))
            .overflow_hidden()
            .child(args);
        if let Some(error) = &step.error {
            let mut error_box = gpui::div()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .p(gpui::px(metrics.inspector_padding))
                .rounded(gpui::px(metrics.input_radius))
                .bg(theme.danger_wash)
                .gap(gpui::px(metrics.control_gap))
                .child(
                    gpui::div()
                        .flex()
                        .items_center()
                        .gap(gpui::px(metrics.control_gap))
                        .child(crate::inspector::colored_icon(
                            "WarningCircle",
                            theme,
                            theme.danger,
                        ))
                        .child(
                            gpui::div()
                                .text_color(theme.danger)
                                .child(label(short(error))),
                        ),
                );
            if let Some(snapshot) = &step.snapshot {
                for key in ["expected", "received"] {
                    if let Some(value) = snapshot.get(key) {
                        error_box = error_box.child(
                            gpui::div()
                                .flex()
                                .gap(gpui::px(metrics.control_gap))
                                .font_family(theme.font_mono.clone())
                                .child(
                                    gpui::div()
                                        .w(gpui::px(metrics.test_argument_width))
                                        .text_color(theme.text_faint)
                                        .child(label(key)),
                                )
                                .child(
                                    gpui::div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_color(if key == "received" {
                                            theme.danger
                                        } else {
                                            theme.text
                                        })
                                        .child(label(short(&value.to_string()))),
                                ),
                        );
                    }
                }
            }
            content = content.child(error_box);
        }
        for error in &step.artifact_errors {
            content = content.child(
                gpui::div()
                    .text_color(theme.danger)
                    .child(label(format!("Artifact: {}", short(error)))),
            );
        }
        let mut tabs = gpui::div()
            .flex()
            .items_center()
            .gap(gpui::px(metrics.control_gap))
            .text_color(theme.text_faint)
            .child(icon("Camera", theme))
            .child(label(if matches!(step.status, Status::Skipped) {
                "Step skipped"
            } else {
                "After action / assertion"
            }))
            .child(gpui::div().flex_1());
        for (title, tree) in [("Snapshot", false), ("Tree", true)] {
            tabs = tabs.child(
                gpui::div()
                    .id(format!("__gpuix_evidence_tab_{title}"))
                    .cursor_pointer()
                    .px(gpui::px(metrics.control_gap))
                    .rounded(gpui::px(metrics.input_radius))
                    .bg(if self.tree_mode == tree {
                        theme.row_pressed
                    } else {
                        theme.bg
                    })
                    .text_color(if self.tree_mode == tree {
                        theme.text
                    } else {
                        theme.text_muted
                    })
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.tree_mode = tree;
                        cx.notify();
                    }))
                    .child(label(title)),
            );
        }
        let mut well = gpui::div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .relative()
            .overflow_hidden()
            .border_1()
            .border_color(theme.border)
            .rounded(gpui::px(metrics.input_radius))
            .bg(theme.field)
            .p(gpui::px(metrics.test_padding));
        if self.tree_mode {
            let tree = if let Some(path) = &step.tree {
                if self
                    .tree_cache
                    .as_ref()
                    .is_none_or(|(cached, _)| cached != path)
                {
                    let text = std::fs::File::open(path)
                        .map_err(|error| error.to_string())
                        .and_then(|file| {
                            let mut bytes = Vec::new();
                            file.take(262145)
                                .read_to_end(&mut bytes)
                                .map_err(|error| error.to_string())?;
                            let truncated = bytes.len() > 262144;
                            bytes.truncate(262144);
                            let mut text = String::from_utf8_lossy(&bytes).into_owned();
                            if truncated {
                                text.push_str(
                                    "\n… Preview truncated; the full tree is in the artifact file.",
                                );
                            }
                            Ok(text)
                        })
                        .unwrap_or_else(|error| format!("Tree unavailable: {error}"));
                    self.tree_cache = Some((path.clone(), text));
                }
                self.tree_cache.as_ref().unwrap().1.clone()
            } else {
                "No tree captured for this step".into()
            };
            well = well.child(
                gpui::div()
                    .id("__gpuix_test_tree")
                    .size_full()
                    .overflow_y_scroll()
                    .font_family(theme.font_mono.clone())
                    .child(label(tree)),
            );
        } else if let Some(path) = &step.screenshot {
            let overlay = step.snapshot.clone();
            let ring_color = if matches!(step.status, Status::Failed) {
                theme.danger
            } else {
                theme.accent
            };
            let ring_width = metrics.control_ring_width;
            well = well.child(
                gpui::div()
                    .relative()
                    .size_full()
                    .child(
                        gpui::img(path.clone())
                            .id(format!(
                                "__gpuix_test_evidence_{}_{}",
                                self.selected_case.unwrap_or_default(),
                                selected_step.unwrap_or_default()
                            ))
                            .size_full()
                            .object_fit(gpui::ObjectFit::Contain)
                            .with_fallback(|| {
                                label("Screenshot unavailable (file missing or unreadable)")
                            }),
                    )
                    .child(
                        gpui::canvas(
                            |bounds, _, _| bounds,
                            move |bounds, _, window, _| {
                                let Some(snapshot) = &overlay else { return };
                                let Some(target) =
                                    snapshot.get("target").filter(|target| target.is_object())
                                else {
                                    return;
                                };
                                let width = snapshot["width"].as_f64().unwrap_or_default() as f32;
                                let height = snapshot["height"].as_f64().unwrap_or_default() as f32;
                                if width <= 0.0 || height <= 0.0 {
                                    return;
                                }
                                let scale = snapshot["scaleFactor"].as_f64().unwrap_or(1.0) as f32;
                                let factor = (f32::from(bounds.size.width) / width)
                                    .min(f32::from(bounds.size.height) / height);
                                let x = f32::from(bounds.origin.x)
                                    + (f32::from(bounds.size.width) - width * factor) / 2.0;
                                let y = f32::from(bounds.origin.y)
                                    + (f32::from(bounds.size.height) - height * factor) / 2.0;
                                let value = |name: &str| {
                                    target[name].as_f64().unwrap_or_default() as f32
                                        * scale
                                        * factor
                                };
                                let target = gpui::Bounds::new(
                                    gpui::point(gpui::px(x + value("x")), gpui::px(y + value("y"))),
                                    gpui::size(gpui::px(value("width")), gpui::px(value("height"))),
                                );
                                window.paint_quad(gpui::quad(
                                    target,
                                    gpui::px(0.0),
                                    gpui::transparent_black(),
                                    gpui::px(ring_width),
                                    ring_color,
                                    gpui::BorderStyle::Solid,
                                ));
                            },
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                    ),
            );
        } else {
            well = well.child(label(if matches!(step.status, Status::Skipped) {
                "Step was skipped; no screenshot captured"
            } else {
                "No screenshot captured for this step"
            }));
        }
        if let Some(test_id) = step.step["testId"].as_str() {
            tabs = tabs.child(
                gpui::div()
                    .font_family(theme.font_mono.clone())
                    .text_color(theme.accent)
                    .child(label(format!("[{test_id}]"))),
            );
        }
        content = content.child(tabs).child(well);
        gpui::div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(header)
            .child(content)
            .into_any_element()
    }
}

impl Render for Results {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        use crate::inspector::{icon, label};
        let theme = crate::theme::Theme::default();
        let metrics = theme.metrics;
        let passed = self
            .cases
            .values()
            .filter(|case| matches!(case.status, Status::Passed))
            .count();
        let failed = self
            .cases
            .values()
            .filter(|case| matches!(case.status, Status::Failed))
            .count();
        let busy = self
            .cases
            .values()
            .any(|case| matches!(case.status, Status::Pending | Status::Running));
        let elapsed = self
            .cases
            .values()
            .map(|case| case.duration_ms)
            .sum::<u64>();
        let visible = self.visible_cases();
        if self.selected_case.is_none_or(|id| !visible.contains(&id)) {
            self.selected_case = visible.first().copied();
            self.selected_step = None;
        }
        let selected_case = self.selected_case;
        let selected_step = self.active_step();
        let mut rows = Vec::new();
        let mut group = None;
        for id in visible {
            let case = &self.cases[&id];
            if group.as_ref() != Some(&case.suite) {
                if let Some(name) = &case.suite {
                    rows.push(TestRow::Group(name.clone()));
                }
                group = Some(case.suite.clone());
            }
            rows.push(TestRow::Test(case.clone()));
        }
        let summary = gpui::div()
            .flex()
            .items_center()
            .h(gpui::px(metrics.test_toolbar_height))
            .flex_shrink_0()
            .px(gpui::px(metrics.test_padding))
            .gap(gpui::px(metrics.control_gap))
            .bg(theme.surface)
            .border_b_1()
            .border_color(theme.border)
            .child(status_icon(
                if busy {
                    Status::Running
                } else if failed > 0 {
                    Status::Failed
                } else {
                    Status::Passed
                },
                &theme,
            ))
            .child(
                gpui::div()
                    .min_w_0()
                    .truncate()
                    .child(label(self.name.clone())),
            )
            .child(
                gpui::div()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .gap(gpui::px(metrics.control_gap))
                    .text_color(theme.text_muted)
                    .child(
                        gpui::div()
                            .size(gpui::px(metrics.test_status_dot_size))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(theme.diff_add),
                    )
                    .child(label(format!("{passed} passed"))),
            )
            .child(
                gpui::div()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .gap(gpui::px(metrics.control_gap))
                    .text_color(if failed > 0 {
                        theme.danger
                    } else {
                        theme.text_faint
                    })
                    .child(
                        gpui::div()
                            .size(gpui::px(metrics.test_status_dot_size))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(if failed > 0 { theme.danger } else { theme.scrollbar_thumb }),
                    )
                    .child(label(format!("{failed} failed"))),
            )
            .child(
                gpui::div()
                    .flex_shrink_0()
                    .font_family(theme.font_mono.clone())
                    .text_color(theme.text_faint)
                    .child(label(format!("{elapsed} ms"))),
            )
            .child(gpui::div().flex_1())
            .child(
                gpui::div()
                    .flex()
                    .items_center()
                    .gap(gpui::px(metrics.control_gap))
                    .w(gpui::px(metrics.inspector_network_filter_width * 2.0))
                    .h(gpui::px(metrics.input_height))
                    .px(gpui::px(metrics.control_gap))
                    .bg(theme.field)
                    .rounded(gpui::px(metrics.input_radius))
                    .border_1()
                    .border_color(theme.border_strong)
                    .child(icon("MagnifyingGlass", &theme))
                    .child(self.filter.render(&theme, window, cx)),
            );
        let mut summary = summary;
        for (title, command) in [("Rerun failed", "failed"), ("Run all", "all")] {
            if command == "failed" && failed == 0 {
                continue;
            }
            summary =
                summary.child(
                    gpui::div()
                        .id(format!("__gpuix_test_run_{command}"))
                        .flex()
                        .items_center()
                        .px(gpui::px(metrics.control_gap))
                        .h(gpui::px(metrics.input_height))
                        .gap(gpui::px(metrics.control_gap))
                        .rounded(gpui::px(metrics.input_radius))
                        .cursor_pointer()
                        .bg(if command == "all" {
                            theme.accent
                        } else {
                            theme.raised
                        })
                        .text_color(if command == "all" {
                            theme.on_accent
                        } else {
                            theme.text_muted
                        })
                        .opacity(if busy { 0.5 } else { 1.0 })
                        .on_click(cx.listener(move |view, _, _, cx| {
                            if !view.cases.values().any(|case| {
                                matches!(case.status, Status::Pending | Status::Running)
                            }) && view.commands.is_empty()
                            {
                                view.commands.push(command.into());
                                cx.notify();
                            }
                        }))
                        .child(label(title)),
                );
        }
        let test_view = cx.weak_entity();
        let test_theme = theme.clone();
        let tests = gpui::uniform_list("__gpuix_test_results", rows.len(), move |range, _, _| {
            range
                .map(|index| {
                    let row = gpui::div()
                        .id(format!("__gpuix_test_row_{index}"))
                        .w_full()
                        .h(gpui::px(metrics.test_row_height))
                        .flex()
                        .items_center()
                        .px(gpui::px(metrics.test_file_padding))
                        .gap(gpui::px(metrics.control_gap))
                        .overflow_hidden();
                    match &rows[index] {
                        TestRow::Group(name) => row
                            .text_color(test_theme.text_muted)
                            .child(icon("CaretDown", &test_theme))
                            .child(icon("FileCode", &test_theme))
                            .child(
                                gpui::div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(test_theme.font_mono.clone())
                                    .child(label(
                                        PathBuf::from(name)
                                            .file_name()
                                            .and_then(|name| name.to_str())
                                            .unwrap_or(name)
                                            .to_string(),
                                    )),
                            ),
                        TestRow::Test(case) => {
                            let view = test_view.clone();
                            let id = case.index;
                            row.pl(gpui::px(metrics.test_file_padding + metrics.test_case_indent))
                                .cursor_pointer()
                                .bg(if selected_case == Some(id) {
                                    test_theme.row_selected
                                } else {
                                    test_theme.surface
                                })
                                .hover(|row| row.bg(test_theme.row_hover))
                                .on_click(move |_, window, cx| {
                                    view.update(cx, |view, cx| {
                                        view.selected_case = Some(id);
                                        view.selected_step = None;
                                        view.tree_cache = None;
                                        view.focus.focus(window, cx);
                                        cx.notify();
                                    })
                                    .ok();
                                })
                                .child(status_icon(case.status, &test_theme))
                                .child(
                                    gpui::div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(label(case.name.clone())),
                                )
                                .child(
                                    gpui::div()
                                        .flex_shrink_0()
                                        .font_family(test_theme.font_mono.clone())
                                        .text_color(test_theme.text_faint)
                                        .child(label(format!("{} ms", case.duration_ms))),
                                )
                        }
                    }
                })
                .collect::<Vec<_>>()
        })
        .track_scroll(&self.case_scroll)
        .flex_1()
        .min_h_0()
        .w_full();
        let selected = self.selected_case.and_then(|id| self.cases.get(&id));
        let title = selected
            .map(|case| case.name.clone())
            .unwrap_or_else(|| "Steps".into());
        let steps = selected.map(|case| case.steps.clone()).unwrap_or_default();
        let step_count = steps.len();
        let max_duration = steps
            .iter()
            .map(|step| step.duration_ms)
            .max()
            .unwrap_or(1)
            .max(1);
        let step_view = cx.weak_entity();
        let step_theme = theme.clone();
        let steps = gpui::uniform_list("__gpuix_test_steps", step_count, move |range, _, _| {
            range
                .map(|index| {
                    let step = &steps[index];
                    let view = step_view.clone();
                    let action = step.step["action"].as_str().unwrap_or("Step");
                    let summary = step.step["name"]
                        .as_str()
                        .or_else(|| step.step["testId"].as_str())
                        .or_else(|| step.step["keys"].as_str())
                        .unwrap_or_default();
                    let color = step.status.appearance(&step_theme).1;
                    gpui::div()
                        .id(format!("__gpuix_test_step_{index}"))
                        .w_full()
                        .flex()
                        .items_center()
                        .h(gpui::px(metrics.test_step_height))
                        .px(gpui::px(metrics.test_step_padding))
                        .gap(gpui::px(metrics.control_gap))
                        .overflow_hidden()
                        .cursor_pointer()
                        .bg(if selected_step == Some(index) {
                            step_theme.row_selected
                        } else {
                            step_theme.bg
                        })
                        .hover(|row| row.bg(step_theme.row_hover))
                        .on_click(move |_, window, cx| {
                            view.update(cx, |view, cx| {
                                view.selected_step = Some(index);
                                view.tree_cache = None;
                                view.focus.focus(window, cx);
                                cx.notify();
                            })
                            .ok();
                        })
                        .child(
                            gpui::div()
                                .flex_shrink_0()
                                .w(gpui::px(metrics.test_step_number_width))
                                .text_right()
                                .text_color(step_theme.text_faint)
                                .font_family(step_theme.font_mono.clone())
                                .child(label((index + 1).to_string())),
                        )
                        .child(status_icon(step.status, &step_theme))
                        .child(
                            gpui::div()
                                .flex()
                                .items_center()
                                .gap(gpui::px(metrics.control_gap))
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .child(
                                    gpui::div()
                                        .flex_shrink_0()
                                        .font_family(step_theme.font_mono.clone())
                                        .text_color(if matches!(step.status, Status::Failed) {
                                            step_theme.danger
                                        } else {
                                            step_theme.text
                                        })
                                        .child(label(action.to_string())),
                                )
                                .child(
                                    gpui::div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_color(step_theme.text_muted)
                                        .truncate()
                                        .child(label(summary.to_string())),
                                ),
                        )
                        .child(
                            gpui::div()
                                .flex_shrink_0()
                                .w(gpui::px(metrics.test_duration_bar_width))
                                .h(gpui::px(metrics.control_ring_width))
                                .rounded(gpui::px(metrics.input_radius))
                                .bg(step_theme.row_hover)
                                .child(
                                    gpui::div()
                                        .h_full()
                                        .w(gpui::relative(
                                            step.duration_ms as f32 / max_duration as f32,
                                        ))
                                        .bg(color),
                                ),
                        )
                        .child(
                            gpui::div()
                                .flex_shrink_0()
                                .w(gpui::px(metrics.test_duration_width))
                                .text_color(step_theme.text_faint)
                                .font_family(step_theme.font_mono.clone())
                                .child(label(format!("{} ms", step.duration_ms))),
                        )
                })
                .collect::<Vec<_>>()
        })
        .track_scroll(&self.step_scroll)
        .flex_1()
        .min_h_0()
        .w_full();
        let details = self.details(&theme, cx);
        gpui::div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .text_color(theme.text)
            .font_family(theme.font_sans.clone())
            .text_size(gpui::px(metrics.inspector_text_size))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "f"
                    && (event.keystroke.modifiers.platform || event.keystroke.modifiers.control)
                {
                    view.filter.focus(window, cx);
                    cx.stop_propagation();
                    return;
                }
                if view.focus.is_focused(window)
                    && matches!(
                        event.keystroke.key.as_str(),
                        "up" | "down" | "left" | "right"
                    )
                {
                    view.navigate(&event.keystroke.key);
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(crate::text::selection_frame_reset(
                self.selection.clone(),
                |_, _| {},
                |_| {},
            ))
            .child(summary)
            .child(
                gpui::div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(
                        gpui::div()
                            .flex()
                            .flex_col()
                            .w(gpui::px(metrics.test_tests_width))
                            .flex_shrink_0()
                            .overflow_hidden()
                            .bg(theme.surface)
                            .border_r_1()
                            .border_color(theme.border)
                            .child(tests),
                    )
                    .child(
                        gpui::div()
                            .flex()
                            .flex_col()
                            .w(gpui::px(metrics.test_steps_width))
                            .flex_shrink_0()
                            .overflow_hidden()
                            .border_r_1()
                            .border_color(theme.border)
                            .child(
                                section(title, &theme).child(gpui::div().flex_1()).child(
                                    gpui::div()
                                        .text_color(theme.text_faint)
                                        .child(label(format!("{step_count} steps"))),
                                ),
                            )
                            .child(steps),
                    )
                    .child(details),
            )
            .child(
                gpui::div()
                    .h(gpui::px(metrics.test_footer_height))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .px(gpui::px(metrics.test_padding))
                    .gap(gpui::px(metrics.test_padding))
                    .bg(theme.surface)
                    .border_t_1()
                    .border_color(theme.border)
                    .text_color(theme.text_faint)
                    .child(label("↑ ↓ steps"))
                    .child(label("← → tests"))
                    .child(gpui::div().flex_1())
                    .child(label(format!("gpx test · {} cases", self.cases.len()))),
            )
    }
}
impl Status {
    fn appearance(self, theme: &crate::theme::Theme) -> (&'static str, gpui::Hsla) {
        match self {
            Self::Pending => ("Pending", theme.text_muted),
            Self::Running => ("Running", theme.accent),
            Self::Passed => ("Passed", theme.diff_add),
            Self::Failed => ("Failed", theme.danger),
            Self::Skipped => ("Skipped", theme.text_faint),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn controls_queue_reruns_and_filtering_and_arrows_use_the_native_input() {
        let mut cx = gpui::TestAppContext::single();
        let metrics = crate::theme::Theme::default().metrics;
        let handle = cx.update(|cx| {
            crate::custom_elements::input::init(cx);
            update(
                "Suite".into(),
                vec![
                    serde_json::from_value(json!({"index":0,"name":"Alpha","status":"failed"}))
                        .unwrap(),
                    serde_json::from_value(json!({"index":1,"name":"Beta","status":"passed"}))
                        .unwrap(),
                ],
                cx,
            )
            .unwrap();
            let handle = cx.global::<ResultsWindow>().0;
            gpui::AnyWindowHandle::from(handle)
                .update(cx, |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
            handle
        });
        cx.simulate_keystrokes(handle.into(), "right");
        cx.update(|cx| {
            handle
                .read_with(cx, |view, _| assert_eq!(view.selected_case, Some(1)))
                .unwrap();
            gpui::AnyWindowHandle::from(handle)
                .update(cx, |_, window, cx| {
                    let width = f32::from(window.viewport_size().width);
                    crate::automation::dispatch_click(
                        window,
                        cx,
                        f64::from(width - metrics.test_padding - metrics.control_gap),
                        f64::from(metrics.test_toolbar_height / 2.0),
                        0,
                        Default::default(),
                    );
                })
                .unwrap();
            assert_eq!(commands(cx).unwrap(), json!(["all"]));
            assert_eq!(commands(cx).unwrap(), json!([]));
        });
        cx.simulate_keystrokes(handle.into(), "cmd-f");
        cx.update(|cx| {
            gpui::AnyWindowHandle::from(handle)
                .update(cx, |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        });
        cx.simulate_input(handle.into(), "Alpha");
        cx.update(|cx| {
            handle
                .read_with(cx, |view, _| assert_eq!(view.visible_cases(), vec![0]))
                .unwrap();
        });
    }

    fn write_evidence(path: &std::path::Path, screenshot: &str) {
        std::fs::write(path, serde_json::to_vec(&json!({"steps":[
            {"step":{"action":"click","name":"Before"},"status":"passed","durationMs":1,"error":null,"screenshot":null},
            {"step":{"action":"expectText","name":"Long step description ".repeat(40)},"status":"passed","durationMs":2,"error":null,"screenshot":screenshot}
        ]})).unwrap()).unwrap();
    }

    #[test]
    fn evidence_resolves_local_screenshots_and_rejects_paths_outside_the_case() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("report.json");
        write_evidence(&path, "step-002.png");
        let steps = evidence(&path).unwrap();
        assert_eq!(
            steps[1].screenshot.as_ref().unwrap(),
            &directory.path().join("step-002.png")
        );
        for invalid in ["../secret.png", "/tmp/secret.png", "nested/image.png"] {
            write_evidence(&path, invalid);
            assert!(evidence(&path).is_err());
        }
        std::fs::write(&path, "not json").unwrap();
        assert!(evidence(&path).is_err());
    }

    #[test]
    fn clicking_a_test_then_a_step_selects_its_saved_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("report.json");
        write_evidence(&path, "step-002.png");
        let cx = gpui::TestAppContext::single();
        let metrics = crate::theme::Theme::default().metrics;
        let handle = cx.update(|cx| {
            update("Suite".into(), (0..2).map(|index| serde_json::from_value(json!({"index":index,"name":if index == 0 { "Long test title ".repeat(40) } else { "Short".into() },"status":"passed","reportPath":path})).unwrap()).collect(), cx).unwrap();
            let handle = cx.global::<ResultsWindow>().0;
            gpui::AnyWindowHandle::from(handle).update(cx, |_, window, cx| window.draw(cx).clear(cx)).unwrap();
            handle
        });
        cx.update(|cx| {
            gpui::AnyWindowHandle::from(handle)
                .update(cx, |_, window, cx| {
                    crate::automation::dispatch_click(
                        window,
                        cx,
                        f64::from(metrics.test_tests_width - metrics.inspector_padding),
                        f64::from(metrics.test_toolbar_height + metrics.test_row_height * 1.5),
                        0,
                        Default::default(),
                    );
                })
                .unwrap();
        });
        cx.update(|cx| {
            handle
                .read_with(cx, |view, _| assert_eq!(view.selected_case, Some(1)))
                .unwrap();
            gpui::AnyWindowHandle::from(handle)
                .update(cx, |_, window, cx| {
                    window.refresh();
                    window.draw(cx).clear(cx);
                    crate::automation::dispatch_click(
                        window,
                        cx,
                        f64::from(metrics.test_tests_width + metrics.test_steps_width - metrics.inspector_padding),
                        f64::from(
                            metrics.test_toolbar_height
                                + metrics.test_section_height
                                + metrics.test_step_height * 1.5,
                        ),
                        0,
                        Default::default(),
                    );
                })
                .unwrap();
        });
        cx.update(|cx| {
            handle
                .read_with(cx, |view, _| {
                    assert_eq!(view.selected_step, Some(1));
                    assert!(view.cases[&1].steps[1]
                        .screenshot
                        .as_ref()
                        .unwrap()
                        .ends_with("step-002.png"));
                })
                .unwrap();
        });
    }
}
