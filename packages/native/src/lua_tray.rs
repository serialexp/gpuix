use crate::script::tray_api::{TrayCommand, TrayItem, TraySpec};
use mlua::{AnyUserData, Function, Lua, Table, UserData};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(crate) struct LuaTrays {
    next_id: u64,
    next_action: u64,
    handlers: HashMap<u64, HashMap<u64, Function>>,
    commands: Vec<TrayCommand>,
}

impl LuaTrays {
    pub(crate) fn take_commands(&mut self) -> Vec<TrayCommand> {
        std::mem::take(&mut self.commands)
    }
    pub(crate) fn handler(&self, action: u64) -> Option<Function> {
        self.handlers
            .values()
            .find_map(|handlers| handlers.get(&action))
            .cloned()
    }
    pub(crate) fn begin_reload(&mut self) {
        self.commands.extend(
            self.handlers
                .keys()
                .map(|id| TrayCommand::Remove { id: *id }),
        );
        self.handlers.clear();
    }
}

struct TrayHandle(u64);
impl UserData for TrayHandle {}

fn options(
    options: Table,
    id: u64,
    root: &Path,
    state: &mut LuaTrays,
) -> mlua::Result<TrayCommand> {
    let icon: String = options.get("icon")?;
    let path = PathBuf::from(icon);
    let icon = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    let tooltip = options
        .get::<Option<String>>("tooltip")?
        .unwrap_or_default();
    let menu: Table = options.get("menu")?;
    let mut items = Vec::new();
    let mut handlers = HashMap::new();
    for item in menu.sequence_values::<Table>() {
        let item = item?;
        if item.get::<Option<bool>>("separator")?.unwrap_or(false) {
            items.push(TrayItem::Separator { separator: true });
        } else {
            let callback: Function = item.get("onClick")?;
            state.next_action += 1;
            let action = state.next_action;
            handlers.insert(action, callback);
            items.push(TrayItem::Action {
                label: item.get("label")?,
                action,
                checked: item.get::<Option<bool>>("checked")?.unwrap_or(false),
                enabled: item.get::<Option<bool>>("enabled")?.unwrap_or(true),
            });
        }
    }
    let command = TrayCommand::Set {
        spec: TraySpec {
            id,
            icon,
            tooltip,
            menu: items,
        },
    };
    command.validate().map_err(mlua::Error::runtime)?;
    state.handlers.insert(id, handlers);
    Ok(command)
}

pub(crate) fn install(
    lua: &Lua,
    api: &Table,
    state: &Arc<Mutex<LuaTrays>>,
    root: PathBuf,
) -> mlua::Result<()> {
    let weak = Arc::downgrade(state);
    let create_root = root.clone();
    api.set(
        "create_tray",
        lua.create_function(move |lua, (app, settings): (AnyUserData, Table)| {
            super::lua_runtime::validate_app_handle(&app)?;
            let state = weak
                .upgrade()
                .ok_or_else(|| mlua::Error::runtime("application has stopped"))?;
            let mut state = state.lock().unwrap();
            state.next_id += 1;
            let id = state.next_id;
            let command = options(settings, id, &create_root, &mut state)?;
            state.commands.push(command);
            lua.create_userdata(TrayHandle(id))
        })?,
    )?;
    let weak = Arc::downgrade(state);
    api.set(
        "update_tray",
        lua.create_function(move |_, (handle, settings): (AnyUserData, Table)| {
            let id = handle.borrow::<TrayHandle>()?.0;
            let state = weak
                .upgrade()
                .ok_or_else(|| mlua::Error::runtime("application has stopped"))?;
            let mut state = state.lock().unwrap();
            if !state.handlers.contains_key(&id) {
                return Err(mlua::Error::runtime("tray has been removed"));
            }
            let command = options(settings, id, &root, &mut state)?;
            state.commands.push(command);
            Ok(())
        })?,
    )?;
    let weak = Arc::downgrade(state);
    api.set(
        "remove_tray",
        lua.create_function(move |_, handle: AnyUserData| {
            let id = handle.borrow::<TrayHandle>()?.0;
            let state = weak
                .upgrade()
                .ok_or_else(|| mlua::Error::runtime("application has stopped"))?;
            let mut state = state.lock().unwrap();
            state.handlers.remove(&id);
            state.commands.push(TrayCommand::Remove { id });
            Ok(())
        })?,
    )?;
    let weak = Arc::downgrade(state);
    api.set(
        "quit",
        lua.create_function(move |_, app: AnyUserData| {
            super::lua_runtime::validate_app_handle(&app)?;
            let state = weak
                .upgrade()
                .ok_or_else(|| mlua::Error::runtime("application has stopped"))?;
            state.lock().unwrap().commands.push(TrayCommand::Quit);
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::lua_runtime::LuaRuntime;
    use crate::retained_tree::RetainedTree;

    fn runtime() -> LuaRuntime {
        let icon = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/tray/icon.svg");
        let source = r#"
            local app = gpuix.create_app()
            gpuix.define_window(app, { id = "main", render = function() return gpuix.text("Tray") end })
            local tray
            local function clicked()
                gpuix.update_tray(tray, {
                    icon = "ICON", tooltip = "Updated", menu = {
                        { label = "Checked", checked = true, enabled = false, onClick = function() gpuix.remove_tray(tray); gpuix.quit(app) end },
                        { separator = true },
                    },
                })
            end
            tray = gpuix.create_tray(app, { icon = "ICON", menu = {{ label = "Click", onClick = clicked }} })
            return app
        "#.replace("ICON", icon.to_str().unwrap());
        LuaRuntime::load(&source, &mut RetainedTree::new()).unwrap()
    }

    #[test]
    fn tray_callbacks_replace_handlers_and_queue_native_updates() {
        let mut runtime = runtime();
        let initial = runtime.take_tray_commands();
        let TrayCommand::Set { spec } = &initial[0] else {
            panic!("expected tray")
        };
        let TrayItem::Action { action, .. } = spec.menu[0] else {
            panic!("expected action")
        };
        runtime.dispatch_tray_action(action).unwrap();
        let updated = runtime.take_tray_commands();
        let TrayCommand::Set { spec: updated } = &updated[0] else {
            panic!("expected update")
        };
        assert_eq!(updated.id, spec.id);
        assert_eq!(updated.tooltip, "Updated");
        assert!(matches!(
            &updated.menu[0],
            TrayItem::Action {
                checked: true,
                enabled: false,
                ..
            }
        ));
        runtime.dispatch_tray_action(action).unwrap();
        assert!(runtime.take_tray_commands().is_empty());
        let TrayItem::Action { action, .. } = updated.menu[0] else {
            panic!("expected action")
        };
        runtime.dispatch_tray_action(action).unwrap();
        assert_eq!(
            runtime.take_tray_commands(),
            vec![TrayCommand::Remove { id: spec.id }, TrayCommand::Quit]
        );
        runtime.dispatch_tray_action(action).unwrap();
        assert!(runtime.take_tray_commands().is_empty());
    }

    #[test]
    fn tray_api_does_not_retain_the_runtime() {
        let lua = Lua::new();
        let api = lua.create_table().unwrap();
        let state = Arc::new(Mutex::new(LuaTrays::default()));
        let weak = Arc::downgrade(&state);
        install(&lua, &api, &state, PathBuf::from(".")).unwrap();
        drop(state);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn reload_replaces_trays_and_rolls_back_failed_entries() {
        let directory = tempfile::tempdir().unwrap();
        let source = include_str!("../../../examples/tray/main.lua");
        let entry = directory.path().join("main.lua");
        let icon = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/tray/icon.svg");
        std::fs::copy(icon, directory.path().join("icon.svg")).unwrap();
        std::fs::write(&entry, source).unwrap();
        let mut runtime = LuaRuntime::load_application_file(&entry).unwrap();
        let initial = runtime.take_tray_commands();
        let TrayCommand::Set { spec } = &initial[0] else {
            panic!("expected tray")
        };
        let TrayItem::Action { action, .. } = spec.menu[1] else {
            panic!("expected toggle")
        };
        std::fs::write(
            &entry,
            source.replace("return app", "error('broken reload')"),
        )
        .unwrap();
        assert!(runtime.reload_application_file(&entry).is_err());
        assert!(runtime.take_tray_commands().is_empty());
        runtime.dispatch_tray_action(action).unwrap();
        let updated = runtime.take_tray_commands();
        assert!(matches!(&updated[0], TrayCommand::Set { spec: updated } if updated.id == spec.id));
        std::fs::write(&entry, source).unwrap();
        runtime.reload_application_file(&entry).unwrap();
        let reloaded = runtime.take_tray_commands();
        assert_eq!(reloaded[0], TrayCommand::Remove { id: spec.id });
        assert!(
            matches!(&reloaded[1], TrayCommand::Set { spec: updated } if updated.id != spec.id)
        );
        runtime.dispatch_tray_action(action).unwrap();
        assert!(runtime.take_tray_commands().is_empty());
    }
}
