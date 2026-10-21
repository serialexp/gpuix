use std::path::PathBuf;

#[derive(Clone, Debug, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraySpec {
    pub id: u64,
    pub icon: PathBuf,
    #[serde(default)]
    pub tooltip: String,
    pub menu: Vec<TrayItem>,
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum TrayItem {
    Separator {
        separator: bool,
    },
    Action {
        label: String,
        action: u64,
        #[serde(default)]
        checked: bool,
        #[serde(default = "enabled")]
        enabled: bool,
    },
}

fn enabled() -> bool {
    true
}

#[derive(Clone, Debug, serde::Deserialize, PartialEq, Eq)]
#[serde(tag = "method", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum TrayCommand {
    Set { spec: TraySpec },
    Remove { id: u64 },
    Quit,
    ShowWindow,
    HideWindow,
}

impl TrayCommand {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !cfg!(target_os = "macos") {
            return Err("application trays are currently supported only on macOS".into());
        }
        if let Self::Set { spec } = self {
            super::tray::validate(Some(&spec.icon))?;
            for item in &spec.menu {
                match item {
                    TrayItem::Separator { separator: true } => {}
                    TrayItem::Action { label, action, .. }
                        if !label.trim().is_empty() && *action > 0 => {}
                    _ => {
                        return Err(
                            "tray items require a nonempty label and callback, or separator = true"
                                .into(),
                        )
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{TrayCommand, TrayItem};
    use gpui::{App, Global, MenuItem};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    #[derive(Clone, Debug, PartialEq, serde::Deserialize, gpui::Action)]
    #[action(namespace = gpuix_tray, no_json)]
    struct Invoke {
        action: u64,
    }

    struct Entry {
        tray: gpui_tray::Tray,
        menu: Rc<RefCell<Vec<TrayItem>>>,
    }
    #[derive(Default)]
    struct Trays(HashMap<u64, Entry>);
    impl Global for Trays {}

    pub(super) fn init(callback: impl Fn(u64) + 'static, cx: &mut App) {
        cx.set_global(Trays::default());
        let callback = Rc::new(callback);
        cx.on_action(move |event: &Invoke, cx| {
            let callback = callback.clone();
            let action = event.action;
            cx.defer(move |_| callback(action));
        });
        cx.on_app_quit(|cx| {
            let trays = cx
                .global_mut::<Trays>()
                .0
                .drain()
                .map(|(_, entry)| entry.tray)
                .collect::<Vec<_>>();
            for tray in trays {
                let _ = tray.close(cx);
            }
            async {}
        })
        .detach();
    }

    fn menu(items: &[TrayItem]) -> Vec<MenuItem> {
        items
            .iter()
            .map(|item| match item {
                TrayItem::Separator { .. } => MenuItem::separator(),
                TrayItem::Action {
                    label,
                    action,
                    checked,
                    enabled,
                } => MenuItem::action(label.clone(), Invoke { action: *action })
                    .checked(*checked)
                    .disabled(!enabled),
            })
            .collect()
    }

    pub(super) fn apply(command: TrayCommand, cx: &mut App) -> Result<(), String> {
        match command {
            TrayCommand::Set { spec } => {
                let image = super::super::tray::load_image(&spec.icon)?;
                let icon =
                    gpui_tray::Icon::from_gpui(&image, cx).map_err(|error| error.to_string())?;
                let existing = cx
                    .global::<Trays>()
                    .0
                    .get(&spec.id)
                    .map(|entry| (entry.tray.clone(), entry.menu.clone()));
                if let Some((tray, items)) = existing {
                    tray.set_icon(Some(icon), cx)
                        .map_err(|error| error.to_string())?;
                    tray.set_tooltip(Some(spec.tooltip), cx)
                        .map_err(|error| error.to_string())?;
                    *items.borrow_mut() = spec.menu;
                    tray.refresh_menu(cx).map_err(|error| error.to_string())?;
                } else {
                    let items = Rc::new(RefCell::new(spec.menu));
                    let menu_items = items.clone();
                    let tray = gpui_tray::Tray::builder()
                        .icon(icon)
                        .tooltip(spec.tooltip)
                        .menu(move |_| menu(&menu_items.borrow()))
                        .build(cx)
                        .map_err(|error| error.to_string())?;
                    cx.global_mut::<Trays>()
                        .0
                        .insert(spec.id, Entry { tray, menu: items });
                }
                cx.set_quit_mode(gpui::QuitMode::Explicit);
            }
            TrayCommand::Remove { id } => {
                let entry = cx.global_mut::<Trays>().0.remove(&id);
                if let Some(entry) = entry {
                    entry.tray.close(cx).map_err(|error| error.to_string())?;
                }
            }
            TrayCommand::Quit => cx.quit(),
            TrayCommand::ShowWindow | TrayCommand::HideWindow => {
                return Err("window commands require an application host".into())
            }
        }
        Ok(())
    }

    pub(super) fn finish(cx: &mut App) {
        let empty = cx.global::<Trays>().0.is_empty();
        cx.set_quit_mode(if empty {
            gpui::QuitMode::LastWindowClosed
        } else {
            gpui::QuitMode::Explicit
        });
        if empty && cx.windows().is_empty() {
            cx.quit();
        }
    }
}

pub(crate) fn finish(cx: &mut gpui::App) {
    #[cfg(target_os = "macos")]
    macos::finish(cx);
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
}

pub(crate) fn init(callback: impl Fn(u64) + 'static, cx: &mut gpui::App) {
    #[cfg(target_os = "macos")]
    macos::init(callback, cx);
    #[cfg(not(target_os = "macos"))]
    let _ = (callback, cx);
}

pub(crate) fn apply(command: TrayCommand, cx: &mut gpui::App) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        macos::apply(command, cx)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (command, cx);
        Err("application trays are currently supported only on macOS".into())
    }
}
