use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mlua::{
    AnyUserData, FromLuaMulti, Function, Lua, LuaSerdeExt, MultiValue, RegistryKey, Table,
    UserData, Value,
};

use crate::element_tree::EventPayload;
use crate::retained_tree::RetainedTree;
use crate::style::StyleDesc;

const ELEMENT_HELPERS: &[(&str, &str)] = &[
    ("div", "div"),
    ("input", "input"),
    ("img", "img"),
    ("svg", "svg"),
    ("code", "code"),
    ("markdown", "markdown"),
    ("diff", "diff"),
    ("anchored", "anchored"),
    ("virtual_list", "virtual-list"),
];

const HOOK_SITE_PREFIX: &str = "__gpuix_hook_site:";

const BUILTIN_LUA_MODULES: &[(&str, &str, bool)] = &[
    (
        "gpuix.solid.controls",
        include_str!("../../lua/gpuix/solid_controls.luax"),
        true,
    ),
    (
        "gpuix.solid.button",
        "return require('gpuix.solid.controls').Button",
        false,
    ),
    (
        "gpuix.solid.checkbox",
        "return require('gpuix.solid.controls').Checkbox",
        false,
    ),
    (
        "gpuix.solid.radio_group",
        "return require('gpuix.solid.controls').RadioGroup",
        false,
    ),
    (
        "gpuix.solid.select",
        "return require('gpuix.solid.controls').Select",
        false,
    ),
    (
        "gpuix.host",
        include_str!("../../lua/gpuix/host.lua"),
        false,
    ),
    (
        "gpuix.solid",
        include_str!("../../lua/gpuix/solid.lua"),
        false,
    ),
    (
        "gpuix._util",
        include_str!("../../lua/gpuix/_util.lua"),
        false,
    ),
    (
        "gpuix.button",
        include_str!("../../lua/gpuix/button.luax"),
        true,
    ),
    (
        "gpuix.checkbox",
        include_str!("../../lua/gpuix/checkbox.luax"),
        true,
    ),
    (
        "gpuix.radio_group",
        include_str!("../../lua/gpuix/radio_group.luax"),
        true,
    ),
    (
        "gpuix.select",
        include_str!("../../lua/gpuix/select.luax"),
        true,
    ),
    (
        "gpuix.combobox",
        include_str!("../../lua/gpuix/combobox.luax"),
        true,
    ),
    (
        "gpuix.drawer",
        include_str!("../../lua/gpuix/drawer.luax"),
        true,
    ),
    (
        "gpuix.dock_layout",
        include_str!("../../lua/gpuix/dock_layout.luax"),
        true,
    ),
    (
        "gpuix.tooltip",
        include_str!("../../lua/gpuix/tooltip.luax"),
        true,
    ),
];

const HANDLE_INDEX_BITS: u32 = 32;
const HANDLE_INDEX_MASK: u64 = u32::MAX as u64;
const HANDLE_MAX_GENERATION: u64 = i32::MAX as u64;

type StyleCache = HashMap<Vec<u8>, Arc<StyleDesc>>;
type HostHandleMap = HashMap<i64, u64>;

#[derive(Clone)]
struct MountedHostHandle {
    root: LuaRootId,
    element_id: u64,
}

type MountedHostHandleMap = HashMap<i64, MountedHostHandle>;

enum HostUpdate {
    Text(i64, String),
    Style(i64, Arc<StyleDesc>),
    Prop(i64, String, serde_json::Value),
    Child(i64, Option<usize>),
    Children(i64, Vec<i64>),
}

impl HostUpdate {
    fn handle(&self) -> i64 {
        match self {
            Self::Text(handle, _)
            | Self::Style(handle, _)
            | Self::Prop(handle, _, _)
            | Self::Child(handle, _)
            | Self::Children(handle, _) => *handle,
        }
    }
}

type HostUpdates = Arc<Mutex<Vec<HostUpdate>>>;
type RootCleanups = Arc<Mutex<HashMap<LuaRootId, Vec<Function>>>>;

#[derive(Clone, Copy)]
struct NodeHandle {
    generation: u64,
    index: usize,
}

#[derive(Clone, Copy)]
struct ChildListHandle {
    generation: u64,
    index: usize,
}

#[derive(Clone)]
struct StyleHandle(Arc<StyleDesc>);

impl UserData for StyleHandle {}

#[derive(Clone, Copy)]
struct LuaAppHandle;

impl UserData for LuaAppHandle {}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LuaWindowOptions {
    pub id: String,
    pub title: String,
    pub width: f32,
    pub height: f32,
    pub open: bool,
    pub focus: bool,
    pub reposition: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LuaAppCommand {
    Open(String),
    Close(String),
    Focus(String),
    SetTitle { id: String, title: String },
}

pub(crate) struct LuaFocusRequest {
    pub root_id: String,
    pub element_id: u64,
}

#[derive(Clone)]
struct LuaWindowDefinition {
    options: LuaWindowOptions,
    render: Function,
}

#[derive(Clone, Default)]
struct LuaApplicationRegistry {
    created: bool,
    definitions: Vec<LuaWindowDefinition>,
    commands: VecDeque<LuaAppCommand>,
}

impl LuaApplicationRegistry {
    fn begin_entry(&mut self) {
        self.created = false;
        self.definitions.clear();
        self.commands.clear();
    }

    fn definition_exists(&self, id: &str) -> bool {
        self.definitions
            .iter()
            .any(|definition| definition.options.id == id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ComponentKey {
    Integer(i64),
    Number(u64),
    String(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ComponentSlot {
    Position(usize),
    Key(ComponentKey),
}

type LuaRootId = Arc<str>;

const DEFAULT_ROOT_ID: &str = "main";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ComponentId {
    root: LuaRootId,
    path: Vec<ComponentSlot>,
}

impl ComponentId {
    fn root(root: LuaRootId) -> Self {
        Self {
            root,
            path: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HookKind {
    State,
    Reducer,
    Ref,
    Memo,
    Callback,
    Effect,
    Store,
    Window,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HookValueType {
    Nil,
    Boolean,
    Number,
    String,
    Table,
    Function,
    Thread,
    UserData,
    LightUserData,
    Error,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HookSignature {
    kind: HookKind,
    value_type: Option<HookValueType>,
    site: Option<u64>,
}

struct HookArguments<T>(T, Option<u64>);

type StateHookArguments = HookArguments<Value>;
type ReducerHookArguments = HookArguments<(Function, Value)>;
type DependencyHookArguments = HookArguments<(Function, Option<Table>)>;
type StoreHookArguments = HookArguments<(Option<Function>, Option<Function>)>;
type WindowHookArguments = HookArguments<(AnyUserData, String)>;

impl<T: FromLuaMulti> FromLuaMulti for HookArguments<T> {
    fn from_lua_multi(mut arguments: MultiValue, lua: &Lua) -> mlua::Result<Self> {
        let site = arguments.back().and_then(|value| match value {
            Value::String(value) => value.to_str().ok().and_then(|value| {
                value
                    .strip_prefix(HOOK_SITE_PREFIX)
                    .and_then(|value| u64::from_str_radix(value, 16).ok())
            }),
            _ => None,
        });
        if site.is_some() {
            arguments.pop_back();
        }
        Ok(Self(T::from_lua_multi(arguments, lua)?, site))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct HookId {
    component: ComponentId,
    index: usize,
}

#[derive(Clone)]
struct DependencyList(Vec<Arc<RegistryKey>>);

#[derive(Clone)]
enum HookSlot {
    State {
        value: Arc<RegistryKey>,
        setter: Arc<RegistryKey>,
    },
    Reducer {
        state: Arc<RegistryKey>,
        reducer: Arc<RegistryKey>,
        dispatch: Arc<RegistryKey>,
    },
    Ref(Arc<RegistryKey>),
    Memo {
        value: Arc<RegistryKey>,
        dependencies: Option<DependencyList>,
    },
    Effect {
        dependencies: Option<DependencyList>,
        cleanup: Option<Arc<RegistryKey>>,
    },
    Store {
        store: String,
        selector: Option<Arc<RegistryKey>>,
        equality: Option<Arc<RegistryKey>>,
        selected: Arc<RegistryKey>,
    },
    Window {
        id: String,
        open: bool,
    },
}

#[derive(Clone, Default)]
struct ComponentHooks {
    signature: Option<Vec<HookSignature>>,
    slots: Vec<HookSlot>,
}

struct PendingEffect {
    id: HookId,
    callback: Arc<RegistryKey>,
    dependencies: Option<DependencyList>,
}

struct EffectJob {
    id: Option<HookId>,
    cleanup: Option<Arc<RegistryKey>>,
    callback: Option<Arc<RegistryKey>>,
}

#[derive(Clone)]
struct StoreSubscriber {
    id: HookId,
    selector: Option<Arc<RegistryKey>>,
    equality: Option<Arc<RegistryKey>>,
    selected: Arc<RegistryKey>,
}

#[derive(Clone)]
struct StoreEntry {
    state: Arc<RegistryKey>,
    reducer: Arc<RegistryKey>,
    listeners: HashMap<u64, Arc<RegistryKey>>,
    next_listener_id: u64,
}

#[derive(Clone, Default)]
struct StoreSnapshot {
    entries: HashMap<String, StoreEntry>,
}

#[derive(Default)]
struct StoreRegistry {
    entries: HashMap<String, StoreEntry>,
    dispatching: HashSet<String>,
}

impl StoreRegistry {
    fn snapshot(&self) -> StoreSnapshot {
        StoreSnapshot {
            entries: self.entries.clone(),
        }
    }

    fn restore(&mut self, snapshot: StoreSnapshot) {
        self.entries = snapshot.entries;
        self.dispatching.clear();
    }

    fn begin_reload(&mut self) {
        for store in self.entries.values_mut() {
            store.listeners.clear();
        }
        self.dispatching.clear();
    }
}

struct HookFrame {
    id: ComponentId,
    signatures: Vec<HookSignature>,
    next_child: usize,
    child_keys: HashSet<ComponentKey>,
    child_slots: Vec<ComponentSlot>,
}

#[derive(Clone)]
struct HookSnapshot {
    components: HashMap<ComponentId, ComponentHooks>,
    dirty_roots: HashSet<LuaRootId>,
}

struct HookStore {
    components: HashMap<ComponentId, ComponentHooks>,
    frames: Vec<HookFrame>,
    seen: HashSet<ComponentId>,
    visited: Vec<ComponentId>,
    mismatch: Option<ComponentId>,
    pending_effects: Vec<PendingEffect>,
    active_root: Option<LuaRootId>,
    refreshing: bool,
    dirty_roots: HashSet<LuaRootId>,
}

impl HookStore {
    fn new() -> Self {
        Self {
            components: HashMap::new(),
            frames: Vec::new(),
            seen: HashSet::new(),
            visited: Vec::new(),
            mismatch: None,
            pending_effects: Vec::new(),
            active_root: None,
            refreshing: false,
            dirty_roots: HashSet::new(),
        }
    }

    fn snapshot(&self) -> HookSnapshot {
        HookSnapshot {
            components: self.components.clone(),
            dirty_roots: self.dirty_roots.clone(),
        }
    }

    fn restore(&mut self, snapshot: HookSnapshot) {
        self.components = snapshot.components;
        self.frames.clear();
        self.seen.clear();
        self.visited.clear();
        self.mismatch = None;
        self.pending_effects.clear();
        self.active_root = None;
        self.refreshing = false;
        self.dirty_roots = snapshot.dirty_roots;
    }

    fn rollback(&mut self, snapshot: HookSnapshot) {
        let mismatch = self.mismatch.take();
        let root = self.active_root.clone();
        self.restore(snapshot);
        self.mismatch = mismatch;
        if let Some(root) = root {
            self.dirty_roots.remove(&root);
        }
    }

    fn begin_render(&mut self, root: LuaRootId, refreshing: bool) {
        self.frames.clear();
        self.seen.clear();
        self.visited.clear();
        self.mismatch = None;
        self.pending_effects.clear();
        self.active_root = Some(root.clone());
        self.refreshing = refreshing;
        self.dirty_roots.remove(&root);
        let root = ComponentId::root(root);
        self.seen.insert(root.clone());
        self.visited.push(root.clone());
        self.frames.push(HookFrame {
            id: root,
            signatures: Vec::new(),
            next_child: 0,
            child_keys: HashSet::new(),
            child_slots: Vec::new(),
        });
    }

    fn begin_component(&mut self, key: Option<ComponentKey>) -> Result<(), String> {
        let parent = self
            .frames
            .last_mut()
            .ok_or_else(|| "Lua component rendered outside the root component".to_string())?;
        let slot = match key {
            Some(key) => {
                if !parent.child_keys.insert(key.clone()) {
                    return Err(format!("duplicate Lua component key {key:?}"));
                }
                ComponentSlot::Key(key)
            }
            None => {
                let position = parent.next_child;
                ComponentSlot::Position(position)
            }
        };
        parent.next_child += 1;
        parent.child_slots.push(slot.clone());
        let mut id = parent.id.clone();
        id.path.push(slot);
        if !self.seen.insert(id.clone()) {
            return Err(format!(
                "Lua component instance {} rendered more than once",
                component_label(&id)
            ));
        }
        self.visited.push(id.clone());
        self.frames.push(HookFrame {
            id,
            signatures: Vec::new(),
            next_child: 0,
            child_keys: HashSet::new(),
            child_slots: Vec::new(),
        });
        Ok(())
    }

    fn abort_component(&mut self) {
        self.frames.pop();
    }

    fn finish_component(&mut self) -> Result<(), String> {
        let frame = self
            .frames
            .pop()
            .ok_or_else(|| "Lua hook frame stack underflow".to_string())?;
        let component = self.components.entry(frame.id.clone()).or_default();
        match &component.signature {
            Some(expected) if expected != &frame.signatures => {
                self.mismatch = Some(frame.id.clone());
                Err(hook_order_error(&frame.id, expected, &frame.signatures))
            }
            Some(_) => Ok(()),
            None => {
                component.signature = Some(frame.signatures);
                Ok(())
            }
        }
    }

    fn next_hook(
        &mut self,
        signature: HookSignature,
    ) -> Result<(HookId, Option<HookSlot>), String> {
        let frame = self
            .frames
            .last_mut()
            .ok_or_else(|| "Lua hook called outside a component render".to_string())?;
        let index = frame.signatures.len();
        let component_id = frame.id.clone();
        if let Some(expected) = self
            .components
            .get(&component_id)
            .and_then(|component| component.signature.as_ref())
        {
            if let Some(expected_signature) = expected.get(index) {
                if expected_signature == &signature {
                    frame.signatures.push(signature);
                } else {
                    self.mismatch = Some(component_id.clone());
                    return Err(hook_signature_error(
                        &component_id,
                        index,
                        *expected_signature,
                        signature,
                    ));
                }
            } else {
                self.mismatch = Some(component_id.clone());
                let mut actual = frame.signatures.clone();
                actual.push(signature);
                return Err(hook_order_error(&component_id, expected, &actual));
            }
        } else {
            frame.signatures.push(signature);
        }
        let slot = self
            .components
            .get(&component_id)
            .and_then(|component| component.slots.get(index))
            .cloned();
        Ok((
            HookId {
                component: component_id,
                index,
            },
            slot,
        ))
    }

    fn push_slot(&mut self, id: &HookId, slot: HookSlot) {
        let component = self.components.entry(id.component.clone()).or_default();
        debug_assert_eq!(component.slots.len(), id.index);
        component.slots.push(slot);
    }

    fn slot(&self, id: &HookId) -> Option<HookSlot> {
        self.components
            .get(&id.component)
            .and_then(|component| component.slots.get(id.index))
            .cloned()
    }

    fn replace_slot(&mut self, id: &HookId, slot: HookSlot) -> Result<(), String> {
        let current = self
            .components
            .get_mut(&id.component)
            .and_then(|component| component.slots.get_mut(id.index))
            .ok_or_else(|| "Lua state setter refers to an unmounted hook".to_string())?;
        *current = slot;
        Ok(())
    }

    fn set_state(&mut self, id: &HookId, value: Arc<RegistryKey>) -> Result<(), String> {
        let current = self
            .components
            .get_mut(&id.component)
            .and_then(|component| component.slots.get_mut(id.index))
            .ok_or_else(|| "Lua state setter refers to an unmounted hook".to_string())?;
        match current {
            HookSlot::State { value: state, .. } => *state = value,
            HookSlot::Reducer { state, .. } => *state = value,
            _ => return Err("Lua state setter refers to a non-state hook".to_string()),
        }
        self.dirty_roots.insert(id.component.root.clone());
        Ok(())
    }

    fn store_subscribers(&self, store: &str) -> Vec<StoreSubscriber> {
        self.components
            .iter()
            .flat_map(|(component_id, component)| {
                component
                    .slots
                    .iter()
                    .enumerate()
                    .filter_map(move |(index, slot)| match slot {
                        HookSlot::Store {
                            store: slot_store,
                            selector,
                            equality,
                            selected,
                        } if slot_store == store => Some(StoreSubscriber {
                            id: HookId {
                                component: component_id.clone(),
                                index,
                            },
                            selector: selector.clone(),
                            equality: equality.clone(),
                            selected: selected.clone(),
                        }),
                        _ => None,
                    })
            })
            .collect()
    }

    fn apply_store_updates(
        &mut self,
        updates: Vec<(HookId, Arc<RegistryKey>)>,
    ) -> HashSet<ComponentId> {
        let mut changed_components = HashSet::new();
        for (id, next) in updates {
            let Some(HookSlot::Store { selected, .. }) = self
                .components
                .get_mut(&id.component)
                .and_then(|component| component.slots.get_mut(id.index))
            else {
                continue;
            };
            *selected = next;
            changed_components.insert(id.component);
        }
        for component in &changed_components {
            self.dirty_roots.insert(component.root.clone());
        }
        changed_components
    }

    fn set_window_open(&mut self, window_id: &str, open: bool) {
        for (component_id, component) in &mut self.components {
            for slot in &mut component.slots {
                let HookSlot::Window { id, open: current } = slot else {
                    continue;
                };
                if id == window_id && *current != open {
                    *current = open;
                    self.dirty_roots.insert(component_id.root.clone());
                }
            }
        }
    }

    fn queue_effect(
        &mut self,
        id: HookId,
        callback: Arc<RegistryKey>,
        dependencies: Option<DependencyList>,
    ) {
        self.pending_effects.push(PendingEffect {
            id,
            callback,
            dependencies,
        });
    }

    fn refreshing(&self) -> bool {
        self.refreshing
    }

    fn take_mismatch(&mut self) -> Option<ComponentId> {
        self.mismatch.take()
    }

    fn reset_component(&mut self, id: &ComponentId) -> Vec<EffectJob> {
        let jobs = self
            .components
            .remove(id)
            .into_iter()
            .flat_map(component_cleanup_jobs)
            .collect();
        self.frames.clear();
        self.mismatch = None;
        self.pending_effects.clear();
        self.dirty_roots.remove(&id.root);
        jobs
    }

    fn component_checkpoint(&self) -> Result<ComponentCheckpoint, String> {
        let frame = self
            .frames
            .last()
            .ok_or_else(|| "gpuix.memo called outside a component render".to_string())?;
        Ok(ComponentCheckpoint {
            child_slot: frame.child_slots.len(),
            visited: self.visited.len(),
        })
    }

    fn component_metadata_since(
        &self,
        checkpoint: ComponentCheckpoint,
    ) -> Result<(Vec<ComponentSlot>, Vec<ComponentId>), String> {
        let frame = self
            .frames
            .last()
            .ok_or_else(|| "gpuix.memo called outside a component render".to_string())?;
        Ok((
            frame.child_slots[checkpoint.child_slot..].to_vec(),
            self.visited[checkpoint.visited..].to_vec(),
        ))
    }

    fn replay_component_metadata(
        &mut self,
        slots: &[ComponentSlot],
        components: &[ComponentId],
    ) -> Result<(), String> {
        let frame = self
            .frames
            .last_mut()
            .ok_or_else(|| "gpuix.memo called outside a component render".to_string())?;
        for slot in slots {
            match slot {
                ComponentSlot::Position(position) if *position != frame.next_child => {
                    return Err(
                        "memoized component positions changed before a cached subtree".to_string(),
                    );
                }
                ComponentSlot::Position(_) => {}
                ComponentSlot::Key(key) if !frame.child_keys.insert(key.clone()) => {
                    return Err(format!("duplicate Lua component key {key:?}"));
                }
                ComponentSlot::Key(_) => {}
            }
            frame.next_child += 1;
            frame.child_slots.push(slot.clone());
        }
        for component in components {
            if !self.seen.insert(component.clone()) {
                return Err(format!(
                    "Lua component instance {} rendered more than once",
                    component_label(component)
                ));
            }
            self.visited.push(component.clone());
        }
        Ok(())
    }

    fn commit_render(&mut self) -> Vec<EffectJob> {
        let root = self
            .active_root
            .take()
            .expect("Lua hook render has an active root");
        let removed = self
            .components
            .extract_if(|component, _| component.root == root && !self.seen.contains(component))
            .map(|(_, hooks)| hooks)
            .collect::<Vec<_>>();
        let mut jobs = removed
            .into_iter()
            .flat_map(component_cleanup_jobs)
            .collect::<Vec<_>>();
        for pending in std::mem::take(&mut self.pending_effects) {
            let Some(HookSlot::Effect {
                dependencies,
                cleanup,
            }) = self
                .components
                .get_mut(&pending.id.component)
                .and_then(|component| component.slots.get_mut(pending.id.index))
            else {
                continue;
            };
            *dependencies = pending.dependencies;
            jobs.push(EffectJob {
                id: Some(pending.id),
                cleanup: cleanup.take(),
                callback: Some(pending.callback),
            });
        }
        self.frames.clear();
        self.visited.clear();
        self.active_root = None;
        self.refreshing = false;
        jobs
    }

    fn set_effect_cleanup(
        &mut self,
        id: &HookId,
        cleanup: Option<Arc<RegistryKey>>,
    ) -> Result<(), String> {
        let slot = self
            .components
            .get_mut(&id.component)
            .and_then(|component| component.slots.get_mut(id.index))
            .ok_or_else(|| "Lua effect belongs to an unmounted component".to_string())?;
        let HookSlot::Effect {
            cleanup: current, ..
        } = slot
        else {
            return Err("Lua effect cleanup refers to a non-effect hook".to_string());
        };
        *current = cleanup;
        Ok(())
    }

    fn take_all_cleanup_jobs(&mut self) -> Vec<EffectJob> {
        std::mem::take(&mut self.components)
            .into_values()
            .flat_map(component_cleanup_jobs)
            .collect()
    }

    fn take_root_cleanup_jobs(&mut self, root: &str) -> Vec<EffectJob> {
        self.dirty_roots.remove(root);
        self.components
            .extract_if(|component, _| component.root.as_ref() == root)
            .map(|(_, hooks)| hooks)
            .flat_map(component_cleanup_jobs)
            .collect()
    }

    fn root_is_dirty(&self, root: &str) -> bool {
        self.dirty_roots.contains(root)
    }

    fn dirty_root_ids(&self) -> Vec<LuaRootId> {
        self.dirty_roots.iter().cloned().collect()
    }

    fn current_root(&self) -> Result<LuaRootId, String> {
        self.active_root
            .clone()
            .ok_or_else(|| "Lua operation requires an active render root".to_string())
    }

    #[cfg(test)]
    fn slot_count(&self) -> usize {
        self.components
            .values()
            .map(|component| component.slots.len())
            .sum()
    }
}

fn component_cleanup_jobs(component: ComponentHooks) -> impl Iterator<Item = EffectJob> {
    component.slots.into_iter().filter_map(|slot| match slot {
        HookSlot::Effect {
            cleanup: Some(cleanup),
            ..
        } => Some(EffectJob {
            id: None,
            cleanup: Some(cleanup),
            callback: None,
        }),
        _ => None,
    })
}

#[derive(Clone, Copy)]
struct ComponentCheckpoint {
    child_slot: usize,
    visited: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum MemoKey {
    Integer(i64),
    Number(u64),
    String(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ScopedMemoKey {
    root: LuaRootId,
    key: MemoKey,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum MemoDependencyKey {
    Boolean(bool),
    Integer(i64),
    Number(u64),
    String(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MemoDependency {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(u64),
    String(String),
    Table(Vec<(MemoDependencyKey, MemoDependency)>),
}

struct MemoEntry {
    dependencies: MemoDependency,
    element_type: Arc<str>,
    key: Option<Arc<str>>,
    component_slots: Arc<[ComponentSlot]>,
    components: Arc<[ComponentId]>,
}

struct MemoHit {
    node: CachedNode,
    component_slots: Arc<[ComponentSlot]>,
    components: Arc<[ComponentId]>,
}

#[derive(Default)]
struct MemoStore {
    entries: HashMap<ScopedMemoKey, MemoEntry>,
    seen: HashSet<ScopedMemoKey>,
}

struct PendingNode {
    element_type: String,
    key: Option<Arc<str>>,
    style: Option<Arc<StyleDesc>>,
    content: Option<String>,
    events: HashMap<String, Function>,
    custom_props: HashMap<String, serde_json::Value>,
    auto_focus: bool,
    test_id: Option<String>,
    children: Vec<usize>,
    memo_key: Option<MemoKey>,
}

struct CachedNode {
    memo_key: MemoKey,
    element_type: Arc<str>,
    key: Option<Arc<str>>,
}

enum ArenaNode {
    Pending(PendingNode),
    Cached(CachedNode),
}

impl ArenaNode {
    fn identity(&self) -> (&str, Option<&str>) {
        match self {
            Self::Pending(node) => (&node.element_type, node.key.as_deref()),
            Self::Cached(node) => (&node.element_type, node.key.as_deref()),
        }
    }

    fn key(&self) -> Option<&Arc<str>> {
        match self {
            Self::Pending(node) => node.key.as_ref(),
            Self::Cached(node) => node.key.as_ref(),
        }
    }
}

#[derive(Default)]
struct RenderArena {
    generation: u64,
    nodes: Vec<Option<ArenaNode>>,
    child_lists: Vec<Option<Vec<usize>>>,
    parented: Vec<bool>,
    sibling_marks: Vec<u64>,
    sibling_epoch: u64,
    sibling_keys: HashSet<Arc<str>>,
}

impl RenderArena {
    fn begin_render(&mut self) {
        self.generation = if self.generation >= HANDLE_MAX_GENERATION {
            1
        } else {
            self.generation + 1
        };
        self.nodes.clear();
        self.child_lists.clear();
        self.parented.clear();
        self.sibling_marks.clear();
        self.sibling_epoch = 0;
        self.sibling_keys.clear();
    }

    fn validate_handle(&self, handle: NodeHandle) -> mlua::Result<()> {
        if handle.generation != self.generation {
            return Err(mlua::Error::runtime(
                "host node came from an earlier render; use gpuix.memo for retained subtrees",
            ));
        }
        if self
            .nodes
            .get(handle.index)
            .and_then(Option::as_ref)
            .is_none()
        {
            return Err(mlua::Error::runtime("host node handle is no longer valid"));
        }
        Ok(())
    }

    fn push_pending(&mut self, node: PendingNode) -> mlua::Result<NodeHandle> {
        self.sibling_epoch = self.sibling_epoch.checked_add(1).unwrap_or_else(|| {
            self.sibling_marks.fill(0);
            1
        });
        self.sibling_keys.clear();
        for child in &node.children {
            let Some(child_node) = self.nodes.get(*child).and_then(Option::as_ref) else {
                return Err(mlua::Error::runtime("host child handle is not valid"));
            };
            let key = child_node.key().cloned();
            if self.sibling_marks[*child] == self.sibling_epoch {
                return Err(mlua::Error::runtime(
                    "the same host node cannot appear twice under one parent",
                ));
            }
            self.sibling_marks[*child] = self.sibling_epoch;
            if self.parented[*child] {
                return Err(mlua::Error::runtime(
                    "a host node cannot belong to more than one parent",
                ));
            }
            if let Some(key) = key {
                if !self.sibling_keys.insert(key.clone()) {
                    return Err(mlua::Error::runtime(format!(
                        "duplicate Lua child key {key:?}"
                    )));
                }
            }
        }
        for child in &node.children {
            self.parented[*child] = true;
        }
        let index = self.nodes.len();
        self.nodes.push(Some(ArenaNode::Pending(node)));
        self.parented.push(false);
        self.sibling_marks.push(0);
        Ok(NodeHandle {
            generation: self.generation,
            index,
        })
    }

    fn push_cached(&mut self, node: CachedNode) -> NodeHandle {
        let index = self.nodes.len();
        self.nodes.push(Some(ArenaNode::Cached(node)));
        self.parented.push(false);
        self.sibling_marks.push(0);
        NodeHandle {
            generation: self.generation,
            index,
        }
    }

    fn push_child_list(&mut self, children: Vec<usize>) -> ChildListHandle {
        let index = self.child_lists.len();
        self.child_lists.push(Some(children));
        ChildListHandle {
            generation: self.generation,
            index,
        }
    }

    fn take_child_list(&mut self, handle: ChildListHandle) -> mlua::Result<Vec<usize>> {
        if handle.generation != self.generation {
            return Err(mlua::Error::runtime(
                "host child list came from an earlier render",
            ));
        }
        self.child_lists
            .get_mut(handle.index)
            .and_then(Option::take)
            .ok_or_else(|| mlua::Error::runtime("host child list was already consumed"))
    }

    fn set_memo_key(&mut self, handle: NodeHandle, key: MemoKey) -> mlua::Result<()> {
        self.validate_handle(handle)?;
        let Some(ArenaNode::Pending(node)) = self.nodes[handle.index].as_mut() else {
            return Err(mlua::Error::runtime(
                "gpuix.memo cannot wrap an already memoized subtree",
            ));
        };
        node.memo_key = Some(key);
        Ok(())
    }

    fn identity(&self, index: usize) -> Result<(String, Option<Arc<str>>), String> {
        let node = self.node(index)?;
        Ok((node.identity().0.to_string(), node.key().cloned()))
    }

    fn identity_matches(&self, index: usize, old: &LuaNode) -> Result<bool, String> {
        let (element_type, key) = self.node(index)?.identity();
        Ok(old.identity_matches(element_type, key))
    }

    fn key(&self, index: usize) -> Result<Option<&str>, String> {
        Ok(self.node(index)?.identity().1)
    }

    fn node(&self, index: usize) -> Result<&ArenaNode, String> {
        self.nodes
            .get(index)
            .and_then(Option::as_ref)
            .ok_or_else(|| format!("Missing staged Lua node {index}"))
    }

    fn take(&mut self, index: usize) -> Result<ArenaNode, String> {
        self.nodes
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(|| format!("Staged Lua node {index} was already consumed"))
    }

    fn validate_root(&self, root: NodeHandle) -> mlua::Result<()> {
        self.validate_handle(root)?;
        if self.parented[root.index] {
            return Err(mlua::Error::runtime(
                "Lua render function returned a node that already has a parent",
            ));
        }
        Ok(())
    }
}

struct LuaNode {
    id: u64,
    handle_token: i64,
    element_type: String,
    key: Option<Arc<str>>,
    events: HashMap<String, Function>,
    children: Vec<LuaNode>,
    memo_key: Option<MemoKey>,
}

struct LuaRoot {
    options: LuaWindowOptions,
    render: Function,
    node: Option<LuaNode>,
    handlers: HashMap<(u64, String), Function>,
    host_tokens: Vec<i64>,
}

impl LuaRoot {
    fn new(options: LuaWindowOptions, render: Function) -> Self {
        Self {
            options,
            render,
            node: None,
            handlers: HashMap::new(),
            host_tokens: Vec::new(),
        }
    }
}

impl LuaNode {
    fn identity_matches(&self, element_type: &str, key: Option<&str>) -> bool {
        self.element_type == element_type && self.key.as_deref() == key
    }
}

pub(crate) struct LuaRuntime {
    lua: Lua,
    roots: HashMap<LuaRootId, LuaRoot>,
    hooks: Arc<Mutex<HookStore>>,
    stores: Arc<Mutex<StoreRegistry>>,
    memo: Arc<Mutex<MemoStore>>,
    arena: Arc<Mutex<RenderArena>>,
    host_handles: Arc<Mutex<MountedHostHandleMap>>,
    host_updates: HostUpdates,
    root_cleanups: RootCleanups,
    focus_request: Arc<Mutex<Option<LuaFocusRequest>>>,
    open_windows: Arc<Mutex<HashSet<String>>>,
    application: Arc<Mutex<LuaApplicationRegistry>>,
    application_entry: bool,
    loaded_modules: Arc<Mutex<HashSet<String>>>,
    next_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReloadOutcome {
    PreservedState,
    ResetState,
}

pub(crate) struct LuaApplicationReload {
    pub windows: Vec<LuaWindowOptions>,
    pub removed: Vec<String>,
}

struct ModuleSnapshot {
    values: Vec<(String, Value)>,
}

fn roots_from_entry(
    entry: Value,
    application: &Arc<Mutex<LuaApplicationRegistry>>,
) -> Result<(HashMap<LuaRootId, LuaRoot>, bool), String> {
    match entry {
        Value::Function(render) => {
            let options = LuaWindowOptions {
                id: DEFAULT_ROOT_ID.to_string(),
                title: "GPUIX".to_string(),
                width: 800.0,
                height: 600.0,
                open: true,
                focus: true,
                reposition: false,
            };
            Ok((
                HashMap::from([(Arc::from(DEFAULT_ROOT_ID), LuaRoot::new(options, render))]),
                false,
            ))
        }
        Value::UserData(handle) => {
            validate_app_handle(&handle).map_err(lua_error)?;
            let application = application.lock().unwrap();
            if !application.created {
                return Err("Lua entry returned an inactive application".to_string());
            }
            if application.definitions.is_empty() {
                return Err("Lua application must define at least one window".to_string());
            }
            Ok((
                application
                    .definitions
                    .iter()
                    .cloned()
                    .map(|definition| {
                        (
                            Arc::from(definition.options.id.as_str()),
                            LuaRoot::new(definition.options, definition.render),
                        )
                    })
                    .collect(),
                true,
            ))
        }
        value => Err(format!(
            "Lua entry must return a render function or gpuix application, got {}",
            value.type_name()
        )),
    }
}

impl LuaRuntime {
    pub(crate) fn load(source: &str, tree: &mut RetainedTree) -> Result<Self, String> {
        Self::load_source(source, None, false, tree)
    }

    pub(crate) fn load_luax(source: &str, tree: &mut RetainedTree) -> Result<Self, String> {
        Self::load_source(source, None, true, tree)
    }

    pub(crate) fn load_file(path: &Path, tree: &mut RetainedTree) -> Result<Self, String> {
        let mut runtime = Self::load_application_file(path)?;
        if runtime.roots.len() != 1 || !runtime.roots.contains_key(DEFAULT_ROOT_ID) {
            return Err(
                "A multiwindow Lua application cannot be mounted in a single test renderer"
                    .to_string(),
            );
        }
        runtime.render(tree, false)?;
        Ok(runtime)
    }

    pub(crate) fn load_application_file(path: &Path) -> Result<Self, String> {
        let source = std::fs::read_to_string(path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        let luax = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("luax"));
        Self::load_source_unrendered(&source, Some(path), luax)
    }

    fn load_source(
        source: &str,
        path: Option<&Path>,
        luax: bool,
        tree: &mut RetainedTree,
    ) -> Result<Self, String> {
        let mut runtime = Self::load_source_unrendered(source, path, luax)?;
        if runtime.roots.len() != 1 || !runtime.roots.contains_key(DEFAULT_ROOT_ID) {
            return Err(
                "A multiwindow Lua application cannot be mounted in a single renderer".to_string(),
            );
        }
        runtime.render(tree, false)?;
        Ok(runtime)
    }

    fn load_source_unrendered(
        source: &str,
        path: Option<&Path>,
        luax: bool,
    ) -> Result<Self, String> {
        let lua = Lua::new();
        let hooks = Arc::new(Mutex::new(HookStore::new()));
        let stores = Arc::new(Mutex::new(StoreRegistry::default()));
        let memo = Arc::new(Mutex::new(MemoStore::default()));
        let arena = Arc::new(Mutex::new(RenderArena::default()));
        let host_handles = Arc::new(Mutex::new(MountedHostHandleMap::new()));
        let host_updates = Arc::new(Mutex::new(Vec::new()));
        let root_cleanups = Arc::new(Mutex::new(HashMap::new()));
        let focus_request = Arc::new(Mutex::new(None));
        let open_windows = Arc::new(Mutex::new(HashSet::new()));
        let application = Arc::new(Mutex::new(LuaApplicationRegistry::default()));
        let styles = Arc::new(Mutex::new(StyleCache::new()));
        let loaded_modules = Arc::new(Mutex::new(HashSet::new()));
        install_api(
            &lua,
            hooks.clone(),
            stores.clone(),
            memo.clone(),
            arena.clone(),
            host_handles.clone(),
            host_updates.clone(),
            root_cleanups.clone(),
            focus_request.clone(),
            open_windows.clone(),
            application.clone(),
            styles,
        )
        .map_err(lua_error)?;
        install_builtin_modules(&lua).map_err(lua_error)?;
        if let Some(root) = path.and_then(Path::parent) {
            install_module_searcher(&lua, root, loaded_modules.clone()).map_err(lua_error)?;
        }
        application.lock().unwrap().begin_entry();
        let entry = compile_entry(&lua, source, path, luax)?
            .call::<Value>(())
            .map_err(lua_error)?;
        let (roots, application_entry) = roots_from_entry(entry, &application)?;
        open_windows.lock().unwrap().extend(
            roots
                .values()
                .filter(|root| root.options.open)
                .map(|root| root.options.id.clone()),
        );
        Ok(Self {
            lua,
            roots,
            hooks,
            stores,
            memo,
            arena,
            host_handles,
            host_updates,
            root_cleanups,
            focus_request,
            open_windows,
            application,
            application_entry,
            loaded_modules,
            next_id: 1,
        })
    }

    pub(crate) fn window_options(&self) -> Vec<LuaWindowOptions> {
        if self.application_entry {
            return self
                .application
                .lock()
                .unwrap()
                .definitions
                .iter()
                .map(|definition| definition.options.clone())
                .collect();
        }
        self.roots
            .values()
            .map(|root| root.options.clone())
            .collect()
    }

    pub(crate) fn is_application_entry(&self) -> bool {
        self.application_entry
    }

    pub(crate) fn mount_window(&mut self, id: &str, tree: &mut RetainedTree) -> Result<(), String> {
        self.render_root(id, tree, false)
    }

    pub(crate) fn unmount_window(&mut self, id: &str) {
        self.cleanup_root(id);
        let jobs = self.hooks.lock().unwrap().take_root_cleanup_jobs(id);
        run_effect_cleanups(&self.lua, &jobs);
        if let Some(root) = self.roots.get_mut(id) {
            root.node = None;
            root.handlers.clear();
            let mut handles = self.host_handles.lock().unwrap();
            for token in root.host_tokens.drain(..) {
                handles.remove(&token);
            }
        }
        self.memo
            .lock()
            .unwrap()
            .entries
            .retain(|key, _| key.root.as_ref() != id);
        self.set_window_open(id, false);
    }

    fn cleanup_root(&self, id: &str) {
        let callbacks = self
            .root_cleanups
            .lock()
            .unwrap()
            .remove(id)
            .unwrap_or_default();
        for callback in callbacks.into_iter().rev() {
            if let Err(error) = callback.call::<()>(()) {
                log::error!("Lua root cleanup failed: {error}");
            }
        }
    }

    pub(crate) fn set_window_open(&self, id: &str, open: bool) {
        let changed = {
            let mut windows = self.open_windows.lock().unwrap();
            if open {
                windows.insert(id.to_string())
            } else {
                windows.remove(id)
            }
        };
        if changed {
            self.hooks.lock().unwrap().set_window_open(id, open);
        }
    }

    pub(crate) fn dirty_window_ids(&self) -> Vec<String> {
        self.hooks
            .lock()
            .unwrap()
            .dirty_root_ids()
            .into_iter()
            .map(|root| root.to_string())
            .collect()
    }

    pub(crate) fn take_app_commands(&self) -> Vec<LuaAppCommand> {
        self.application
            .lock()
            .unwrap()
            .commands
            .drain(..)
            .collect()
    }

    pub(crate) fn reload_file(
        &mut self,
        path: &Path,
        tree: &mut RetainedTree,
    ) -> Result<ReloadOutcome, String> {
        let source = std::fs::read_to_string(path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        let luax = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("luax"));
        let entry = compile_entry(&self.lua, &source, Some(path), luax)?;
        let hook_snapshot = self.hooks.lock().unwrap().snapshot();
        let store_snapshot = self.stores.lock().unwrap().snapshot();
        let module_snapshot = self.unload_modules().map_err(lua_error)?;
        self.stores.lock().unwrap().begin_reload();
        let render = match entry.call::<Function>(()) {
            Ok(render) => render,
            Err(error) => {
                self.stores.lock().unwrap().restore(store_snapshot);
                self.restore_modules(module_snapshot).map_err(lua_error)?;
                return Err(lua_error(error));
            }
        };
        let previous_render = {
            let root = self
                .roots
                .get_mut(DEFAULT_ROOT_ID)
                .ok_or_else(|| "Lua main root is missing".to_string())?;
            std::mem::replace(&mut root.render, render)
        };
        self.memo.lock().unwrap().entries.clear();

        let mut reset_components = HashSet::new();
        loop {
            match self.render(tree, true) {
                Ok(()) if reset_components.is_empty() => {
                    return Ok(ReloadOutcome::PreservedState);
                }
                Ok(()) => return Ok(ReloadOutcome::ResetState),
                Err(error) if is_hook_order_error(&error) => {
                    let mismatch = self.hooks.lock().unwrap().take_mismatch();
                    let Some(mismatch) = mismatch else {
                        self.roots.get_mut(DEFAULT_ROOT_ID).unwrap().render = previous_render;
                        self.hooks.lock().unwrap().restore(hook_snapshot);
                        self.stores.lock().unwrap().restore(store_snapshot);
                        self.restore_modules(module_snapshot).map_err(lua_error)?;
                        return Err(error);
                    };
                    if !reset_components.insert(mismatch.clone()) {
                        self.roots.get_mut(DEFAULT_ROOT_ID).unwrap().render = previous_render;
                        self.hooks.lock().unwrap().restore(hook_snapshot);
                        self.stores.lock().unwrap().restore(store_snapshot);
                        self.restore_modules(module_snapshot).map_err(lua_error)?;
                        return Err(error);
                    }
                    let cleanup = self.hooks.lock().unwrap().reset_component(&mismatch);
                    run_effect_cleanups(&self.lua, &cleanup);
                    self.memo.lock().unwrap().entries.clear();
                }
                Err(error) => {
                    self.roots.get_mut(DEFAULT_ROOT_ID).unwrap().render = previous_render;
                    self.hooks.lock().unwrap().restore(hook_snapshot);
                    self.stores.lock().unwrap().restore(store_snapshot);
                    self.restore_modules(module_snapshot).map_err(lua_error)?;
                    return Err(error);
                }
            }
        }
    }

    pub(crate) fn reload_application_file(
        &mut self,
        path: &Path,
    ) -> Result<LuaApplicationReload, String> {
        let source = std::fs::read_to_string(path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        let luax = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("luax"));
        let entry = compile_entry(&self.lua, &source, Some(path), luax)?;
        let store_snapshot = self.stores.lock().unwrap().snapshot();
        let application_snapshot = self.application.lock().unwrap().clone();
        let module_snapshot = self.unload_modules().map_err(lua_error)?;
        self.stores.lock().unwrap().begin_reload();
        self.application.lock().unwrap().begin_entry();
        let value = match entry.call::<Value>(()) {
            Ok(value) => value,
            Err(error) => {
                self.stores.lock().unwrap().restore(store_snapshot);
                *self.application.lock().unwrap() = application_snapshot;
                self.restore_modules(module_snapshot).map_err(lua_error)?;
                return Err(lua_error(error));
            }
        };
        let (mut next_roots, application_entry) = match roots_from_entry(value, &self.application) {
            Ok(roots) => roots,
            Err(error) => {
                self.stores.lock().unwrap().restore(store_snapshot);
                *self.application.lock().unwrap() = application_snapshot;
                self.restore_modules(module_snapshot).map_err(lua_error)?;
                return Err(error);
            }
        };
        if !application_entry {
            self.stores.lock().unwrap().restore(store_snapshot);
            *self.application.lock().unwrap() = application_snapshot;
            self.restore_modules(module_snapshot).map_err(lua_error)?;
            return Err(
                "A multiwindow application cannot reload as a single render function".to_string(),
            );
        }

        let previous_ids = self.roots.keys().cloned().collect::<HashSet<_>>();
        {
            let mut open_windows = self.open_windows.lock().unwrap();
            open_windows.retain(|id| next_roots.contains_key(id.as_str()));
            for (id, root) in &next_roots {
                if !previous_ids.contains(id) && root.options.open {
                    open_windows.insert(id.to_string());
                }
            }
        }

        let mut previous_roots = std::mem::take(&mut self.roots);
        for (id, next) in &mut next_roots {
            let Some(previous) = previous_roots.remove(id.as_ref()) else {
                continue;
            };
            next.node = previous.node;
            next.handlers = previous.handlers;
            next.host_tokens = previous.host_tokens;
        }
        let mut removed = Vec::with_capacity(previous_roots.len());
        for (id, root) in previous_roots {
            let id = id.to_string();
            {
                let mut handles = self.host_handles.lock().unwrap();
                for token in root.host_tokens {
                    handles.remove(&token);
                }
            }
            let jobs = self.hooks.lock().unwrap().take_root_cleanup_jobs(&id);
            run_effect_cleanups(&self.lua, &jobs);
            self.memo
                .lock()
                .unwrap()
                .entries
                .retain(|key, _| key.root.as_ref() != id.as_str());
            removed.push(id);
        }
        self.roots = next_roots;
        self.application_entry = true;
        self.memo.lock().unwrap().entries.clear();
        Ok(LuaApplicationReload {
            windows: self.window_options(),
            removed,
        })
    }

    pub(crate) fn refresh_window(
        &mut self,
        id: &str,
        tree: &mut RetainedTree,
    ) -> Result<ReloadOutcome, String> {
        let mut reset_components = HashSet::new();
        loop {
            match self.render_root(id, tree, true) {
                Ok(()) if reset_components.is_empty() => {
                    return Ok(ReloadOutcome::PreservedState);
                }
                Ok(()) => return Ok(ReloadOutcome::ResetState),
                Err(error) if is_hook_order_error(&error) => {
                    let mismatch = self.hooks.lock().unwrap().take_mismatch();
                    let Some(mismatch) = mismatch else {
                        return Err(error);
                    };
                    if mismatch.root.as_ref() != id || !reset_components.insert(mismatch.clone()) {
                        return Err(error);
                    }
                    let cleanup = self.hooks.lock().unwrap().reset_component(&mismatch);
                    run_effect_cleanups(&self.lua, &cleanup);
                    self.memo
                        .lock()
                        .unwrap()
                        .entries
                        .retain(|key, _| key.root.as_ref() != id);
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub(crate) fn dispatch_event(
        &mut self,
        payload: EventPayload,
        tree: &mut RetainedTree,
    ) -> Result<bool, String> {
        let dirty_roots = self.dispatch_window_event(DEFAULT_ROOT_ID, payload)?;
        let dirty = dirty_roots.iter().any(|root| root == DEFAULT_ROOT_ID);
        if dirty {
            self.render(tree, false)?;
        }
        Ok(dirty)
    }

    pub(crate) fn dispatch_window_event(
        &mut self,
        root_id: &str,
        payload: EventPayload,
    ) -> Result<Vec<String>, String> {
        *self.focus_request.lock().unwrap() = None;
        let id = payload.element_id as u64;
        let Some(handler) = self
            .roots
            .get(root_id)
            .and_then(|root| root.handlers.get(&(id, payload.event_type.clone())))
            .cloned()
        else {
            return Ok(Vec::new());
        };
        let event = event_table(&self.lua, &payload).map_err(lua_error)?;
        if self.host_updates.lock().unwrap().is_empty() {
            self.arena.lock().unwrap().begin_render();
        }
        handler.call::<()>(event).map_err(lua_error)?;
        let mut dirty: HashSet<String> = self
            .hooks
            .lock()
            .unwrap()
            .dirty_root_ids()
            .into_iter()
            .map(|root| root.to_string())
            .collect();
        let handles = self.host_handles.lock().unwrap();
        for update in self.host_updates.lock().unwrap().iter() {
            if let Some(handle) = handles.get(&update.handle()) {
                dirty.insert(handle.root.to_string());
            }
        }
        Ok(dirty.into_iter().collect())
    }

    pub(crate) fn take_focus_request(&self) -> Option<LuaFocusRequest> {
        self.focus_request.lock().unwrap().take()
    }

    fn render(&mut self, tree: &mut RetainedTree, refreshing: bool) -> Result<(), String> {
        self.render_root(DEFAULT_ROOT_ID, tree, refreshing)
    }

    fn render_root(
        &mut self,
        root_id: &str,
        tree: &mut RetainedTree,
        refreshing: bool,
    ) -> Result<(), String> {
        if !refreshing
            && self
                .roots
                .get(root_id)
                .is_some_and(|root| root.node.is_some())
            && !self.hooks.lock().unwrap().root_is_dirty(root_id)
            && !self.host_updates.lock().unwrap().is_empty()
        {
            self.flush_host_updates(root_id, tree)?;
            return Ok(());
        }
        for pass in 0..25 {
            self.render_root_once(root_id, tree, refreshing && pass == 0)?;
            self.flush_host_updates(root_id, tree)?;
            if !self.hooks.lock().unwrap().root_is_dirty(root_id) {
                return Ok(());
            }
        }
        Err(format!(
            "Lua hooks scheduled too many consecutive renders in window {root_id:?}"
        ))
    }

    fn flush_host_updates(&mut self, root_id: &str, tree: &mut RetainedTree) -> Result<(), String> {
        let mut updates = std::mem::take(&mut *self.host_updates.lock().unwrap());
        loop {
            let mut deferred = Vec::new();
            let mut progressed = false;
            for update in updates {
                if matches!(update, HostUpdate::Child(_, _) | HostUpdate::Children(_, _)) {
                    let token = update.handle();
                    let handle = self.host_handles.lock().unwrap().get(&token).cloned();
                    if let Some(handle) = handle.filter(|handle| handle.root.as_ref() == root_id) {
                        match update {
                            HostUpdate::Child(_, index) => {
                                self.replace_host_child(root_id, handle.element_id, index, tree)?
                            }
                            HostUpdate::Children(_, children) => self.replace_host_children(
                                root_id,
                                handle.element_id,
                                children,
                                tree,
                            )?,
                            _ => unreachable!(),
                        }
                        progressed = true;
                    } else {
                        deferred.push(update);
                    }
                } else {
                    deferred.push(update);
                }
            }
            updates = deferred;
            if !progressed {
                break;
            }
        }
        let handles = self.host_handles.lock().unwrap();
        let mut remaining = Vec::new();
        for update in updates {
            let Some(handle) = handles.get(&update.handle()) else {
                let token = Value::Integer(update.handle());
                if node_handle(&token)
                    .and_then(|handle| self.arena.lock().unwrap().validate_handle(handle))
                    .is_ok()
                {
                    remaining.push(update);
                }
                continue;
            };
            if handle.root.as_ref() != root_id {
                remaining.push(update);
                continue;
            }
            match update {
                HostUpdate::Text(_, content) => tree.set_text(handle.element_id, content),
                HostUpdate::Style(_, style) => tree.set_style(handle.element_id, style),
                HostUpdate::Prop(_, key, value) => {
                    tree.set_custom_prop(handle.element_id, key, value)
                }
                HostUpdate::Child(_, _) | HostUpdate::Children(_, _) => {}
            }
        }
        self.host_updates.lock().unwrap().extend(remaining);
        Ok(())
    }

    fn replace_host_child(
        &mut self,
        root_id: &str,
        parent_id: u64,
        index: Option<usize>,
        tree: &mut RetainedTree,
    ) -> Result<(), String> {
        let root = self.roots.get_mut(root_id).ok_or("missing root")?;
        let node = find_host_node(root.node.as_mut().ok_or("unmounted root")?, parent_id)
            .ok_or("missing structural slot")?;
        for child in node.children.drain(..) {
            tree.destroy_element(child.id);
        }
        if let Some(index) = index {
            let child = reconcile_node(
                tree,
                &mut self.next_id,
                None,
                &mut self.arena.lock().unwrap(),
                index,
                &mut HostHandleMap::new(),
            )?;
            tree.append_child(parent_id, child.id);
            node.children.push(child);
        }
        let mut aliases = HostHandleMap::new();
        collect_host_handles(root.node.as_ref().unwrap(), &mut aliases);
        let mut handles = self.host_handles.lock().unwrap();
        for token in root.host_tokens.drain(..) {
            handles.remove(&token);
        }
        root.host_tokens = aliases.keys().copied().collect();
        for (token, element_id) in aliases {
            handles.insert(
                token,
                MountedHostHandle {
                    root: Arc::from(root_id),
                    element_id,
                },
            );
        }
        root.handlers.clear();
        collect_handlers(root.node.as_ref().unwrap(), &mut root.handlers);
        Ok(())
    }

    fn replace_host_children(
        &mut self,
        root_id: &str,
        parent_id: u64,
        tokens: Vec<i64>,
        tree: &mut RetainedTree,
    ) -> Result<(), String> {
        let root = self.roots.get_mut(root_id).ok_or("missing root")?;
        let node = find_host_node(root.node.as_mut().ok_or("unmounted root")?, parent_id)
            .ok_or("missing structural slot")?;
        let mut arena = self.arena.lock().unwrap();
        let mut seen = HashSet::new();
        let existing: HashSet<_> = node
            .children
            .iter()
            .map(|child| child.handle_token)
            .collect();
        for token in &tokens {
            if !seen.insert(*token) {
                return Err("duplicate child handle".into());
            }
            if !existing.contains(token) {
                arena
                    .validate_root(node_handle(&Value::Integer(*token)).map_err(lua_error)?)
                    .map_err(lua_error)?;
            }
        }
        let mut previous: HashMap<_, _> = node
            .children
            .drain(..)
            .map(|child| (child.handle_token, child))
            .collect();
        for token in tokens {
            let child = if let Some(child) = previous.remove(&token) {
                child
            } else {
                let handle = node_handle(&Value::Integer(token)).map_err(lua_error)?;
                reconcile_node(
                    tree,
                    &mut self.next_id,
                    None,
                    &mut arena,
                    handle.index,
                    &mut HostHandleMap::new(),
                )?
            };
            tree.append_child(parent_id, child.id);
            node.children.push(child);
        }
        for child in previous.into_values() {
            tree.destroy_element(child.id);
        }
        let mut aliases = HostHandleMap::new();
        collect_host_handles(root.node.as_ref().unwrap(), &mut aliases);
        let mut handles = self.host_handles.lock().unwrap();
        for token in root.host_tokens.drain(..) {
            handles.remove(&token);
        }
        root.host_tokens = aliases.keys().copied().collect();
        for (token, element_id) in aliases {
            handles.insert(
                token,
                MountedHostHandle {
                    root: Arc::from(root_id),
                    element_id,
                },
            );
        }
        root.handlers.clear();
        collect_handlers(root.node.as_ref().unwrap(), &mut root.handlers);
        Ok(())
    }

    fn render_root_once(
        &mut self,
        root_id: &str,
        tree: &mut RetainedTree,
        refreshing: bool,
    ) -> Result<(), String> {
        let root_key: LuaRootId = self
            .roots
            .get_key_value(root_id)
            .map(|(id, _)| id.clone())
            .ok_or_else(|| format!("Lua window {root_id:?} is not defined"))?;
        let render = self.roots[root_id].render.clone();
        self.cleanup_root(root_id);
        let hook_snapshot = {
            let mut hooks = self.hooks.lock().unwrap();
            let snapshot = hooks.snapshot();
            hooks.begin_render(root_key.clone(), refreshing);
            snapshot
        };
        self.memo.lock().unwrap().seen.clear();
        self.arena.lock().unwrap().begin_render();

        let value = match render.call::<Value>(()) {
            Ok(value) => value,
            Err(error) => {
                self.memo.lock().unwrap().entries.clear();
                self.hooks.lock().unwrap().rollback(hook_snapshot);
                return Err(lua_error(error));
            }
        };
        let hook_result = { self.hooks.lock().unwrap().finish_component() };
        if let Err(error) = hook_result {
            self.memo.lock().unwrap().entries.clear();
            self.hooks.lock().unwrap().rollback(hook_snapshot);
            return Err(error);
        }
        let root_handle = match node_handle(&value).map_err(lua_error) {
            Ok(handle) => handle,
            Err(error) => {
                self.memo.lock().unwrap().entries.clear();
                self.hooks.lock().unwrap().rollback(hook_snapshot);
                return Err(error);
            }
        };
        if let Err(error) = self
            .arena
            .lock()
            .unwrap()
            .validate_root(root_handle)
            .map_err(lua_error)
        {
            self.memo.lock().unwrap().entries.clear();
            self.hooks.lock().unwrap().rollback(hook_snapshot);
            return Err(error);
        }

        {
            let mut memo = self.memo.lock().unwrap();
            let MemoStore { entries, seen } = &mut *memo;
            entries.retain(|key, _| key.root != root_key || seen.contains(key));
        }

        let (old, old_host_tokens) = {
            let root = self.roots.get_mut(root_id).unwrap();
            (root.node.take(), std::mem::take(&mut root.host_tokens))
        };
        let arena = self.arena.clone();
        let mut arena = arena.lock().unwrap();
        let mut handle_aliases = HostHandleMap::new();
        let root = match reconcile_node(
            tree,
            &mut self.next_id,
            old,
            &mut arena,
            root_handle.index,
            &mut handle_aliases,
        ) {
            Ok(root) => root,
            Err(error) => {
                drop(arena);
                self.hooks.lock().unwrap().rollback(hook_snapshot);
                return Err(error);
            }
        };
        drop(arena);
        tree.set_root(Some(root.id));
        collect_host_handles(&root, &mut handle_aliases);
        let host_tokens = handle_aliases.keys().copied().collect();
        {
            let mut handles = self.host_handles.lock().unwrap();
            for token in old_host_tokens {
                handles.remove(&token);
            }
            handles.extend(handle_aliases.into_iter().map(|(token, element_id)| {
                (
                    token,
                    MountedHostHandle {
                        root: root_key.clone(),
                        element_id,
                    },
                )
            }));
        }
        let mut handlers = HashMap::new();
        collect_handlers(&root, &mut handlers);
        let root_state = self.roots.get_mut(root_id).unwrap();
        root_state.node = Some(root);
        root_state.handlers = handlers;
        root_state.host_tokens = host_tokens;
        let effects = self.hooks.lock().unwrap().commit_render();
        self.run_effect_jobs(effects);
        Ok(())
    }

    fn run_effect_jobs(&mut self, jobs: Vec<EffectJob>) {
        run_effect_cleanups(&self.lua, &jobs);
        for job in jobs {
            let (Some(id), Some(callback)) = (job.id, job.callback) else {
                continue;
            };
            let callback = match self.lua.registry_value::<Function>(callback.as_ref()) {
                Ok(callback) => callback,
                Err(error) => {
                    eprintln!("gpuix-lua effect error: {error}");
                    continue;
                }
            };
            let cleanup = match callback.call::<Value>(()) {
                Ok(Value::Nil) => None,
                Ok(Value::Function(cleanup)) => match self.lua.create_registry_value(cleanup) {
                    Ok(cleanup) => Some(Arc::new(cleanup)),
                    Err(error) => {
                        eprintln!("gpuix-lua effect error: {error}");
                        continue;
                    }
                },
                Ok(value) => {
                    eprintln!(
                        "gpuix.use_effect must return a cleanup function or nil, got {}",
                        value.type_name()
                    );
                    None
                }
                Err(error) => {
                    eprintln!("gpuix-lua effect error: {error}");
                    None
                }
            };
            if let Err(error) = self.hooks.lock().unwrap().set_effect_cleanup(&id, cleanup) {
                eprintln!("gpuix-lua effect error: {error}");
            }
        }
    }

    fn unload_modules(&self) -> mlua::Result<ModuleSnapshot> {
        let names = std::mem::take(&mut *self.loaded_modules.lock().unwrap());
        let package: Table = self.lua.globals().get("package")?;
        let loaded: Table = package.get("loaded")?;
        let mut values = Vec::with_capacity(names.len());
        for name in names {
            let value = loaded.raw_get::<Value>(name.as_str())?;
            values.push((name.clone(), value));
            loaded.raw_set(name, Value::Nil)?;
        }
        Ok(ModuleSnapshot { values })
    }

    fn restore_modules(&self, snapshot: ModuleSnapshot) -> mlua::Result<()> {
        let new_names = std::mem::take(&mut *self.loaded_modules.lock().unwrap());
        let package: Table = self.lua.globals().get("package")?;
        let loaded: Table = package.get("loaded")?;
        for name in new_names {
            loaded.raw_set(name, Value::Nil)?;
        }
        let mut old_names = HashSet::with_capacity(snapshot.values.len());
        for (name, value) in snapshot.values {
            loaded.raw_set(name.as_str(), value)?;
            old_names.insert(name);
        }
        *self.loaded_modules.lock().unwrap() = old_names;
        Ok(())
    }
}

impl Drop for LuaRuntime {
    fn drop(&mut self) {
        let roots: Vec<_> = self.roots.keys().cloned().collect();
        for root in roots {
            self.cleanup_root(&root);
        }
        let jobs = self.hooks.lock().unwrap().take_all_cleanup_jobs();
        run_effect_cleanups(&self.lua, &jobs);
    }
}

fn run_effect_cleanups(lua: &Lua, jobs: &[EffectJob]) {
    for job in jobs {
        let Some(cleanup) = &job.cleanup else {
            continue;
        };
        if let Err(error) = lua
            .registry_value::<Function>(cleanup.as_ref())
            .and_then(|cleanup| cleanup.call::<()>(()))
        {
            eprintln!("gpuix-lua effect cleanup error: {error}");
        }
    }
}

fn install_builtin_modules(lua: &Lua) -> mlua::Result<()> {
    let package: Table = lua.globals().get("package")?;
    let preload: Table = package.get("preload")?;
    for &(name, source, luax) in BUILTIN_LUA_MODULES {
        let source = if luax {
            crate::luax::transform(source).map_err(mlua::Error::runtime)?
        } else {
            source.to_string()
        };
        let loader = lua
            .load(&source)
            .set_name(format!("@{name}"))
            .into_function()?;
        preload.set(name, loader)?;
    }
    Ok(())
}

fn compile_entry(
    lua: &Lua,
    source: &str,
    path: Option<&Path>,
    luax: bool,
) -> Result<Function, String> {
    let source = if luax {
        crate::luax::transform(source)?
    } else {
        source.to_string()
    };
    let chunk_name = path
        .map(|path| format!("@{}", path.display()))
        .unwrap_or_else(|| "=gpuix-lua".to_string());
    lua.load(&source)
        .set_name(chunk_name)
        .into_function()
        .map_err(lua_error)
}

fn install_module_searcher(
    lua: &Lua,
    root: &Path,
    loaded_modules: Arc<Mutex<HashSet<String>>>,
) -> mlua::Result<()> {
    let root = root.to_path_buf();
    let searcher = lua.create_function(move |lua, module: String| {
        let candidates = module_candidates(&root, &module).map_err(mlua::Error::runtime)?;
        for path in &candidates {
            if !path.is_file() {
                continue;
            }
            let source = std::fs::read_to_string(path).map_err(mlua::Error::external)?;
            let source = if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("luax"))
            {
                crate::luax::transform(&source).map_err(mlua::Error::runtime)?
            } else {
                source
            };
            let loader = lua
                .load(&source)
                .set_name(format!("@{}", path.display()))
                .into_function()?;
            loaded_modules.lock().unwrap().insert(module);
            return Ok((Value::Function(loader), path.display().to_string()));
        }

        let searched = candidates
            .iter()
            .map(|path| format!("\n\tno file '{}'", path.display()))
            .collect::<String>();
        Ok((Value::String(lua.create_string(searched)?), String::new()))
    })?;
    let package: Table = lua.globals().get("package")?;
    let searchers: Table = package
        .get("searchers")
        .or_else(|_| package.get("loaders"))?;
    searchers.raw_insert(2, searcher)
}

fn module_candidates(root: &Path, module: &str) -> Result<Vec<PathBuf>, String> {
    if module.is_empty()
        || module.split('.').any(|segment| {
            segment.is_empty() || segment == ".." || segment.contains('/') || segment.contains('\\')
        })
    {
        return Err(format!("invalid Lua module name {module:?}"));
    }
    let relative = module.replace('.', std::path::MAIN_SEPARATOR_STR);
    Ok([
        root.join(format!("{relative}.lua")),
        root.join(format!("{relative}.luax")),
        root.join(&relative).join("init.lua"),
        root.join(relative).join("init.luax"),
    ]
    .into())
}

fn is_hook_order_error(error: &str) -> bool {
    error.contains("Lua hook order changed:")
}

fn hook_order_error(
    component: &ComponentId,
    expected: &[HookSignature],
    actual: &[HookSignature],
) -> String {
    let component = component_label(component);
    if expected.len() != actual.len() {
        return format!(
            "Lua hook order changed: component {component} rendered {} hooks, but the previous render used {}. Hooks must not be called conditionally or after an early return",
            actual.len(),
            expected.len()
        );
    }
    let mismatch = expected
        .iter()
        .zip(actual)
        .position(|(expected, actual)| expected != actual)
        .unwrap_or(0);
    format!(
        "Lua hook order changed: component {component} hook {} changed from {} to {}",
        mismatch + 1,
        hook_signature_label(expected[mismatch]),
        hook_signature_label(actual[mismatch])
    )
}

fn hook_signature_error(
    component: &ComponentId,
    index: usize,
    expected: HookSignature,
    actual: HookSignature,
) -> String {
    format!(
        "Lua hook order changed: component {} hook {} changed from {} to {}",
        component_label(component),
        index + 1,
        hook_signature_label(expected),
        hook_signature_label(actual)
    )
}

fn component_label(component: &ComponentId) -> String {
    let mut label = format!("window:{}", component.root);
    if component.path.is_empty() {
        return label;
    }
    for slot in &component.path {
        match slot {
            ComponentSlot::Position(position) => label.push_str(&format!("/#{position}")),
            ComponentSlot::Key(ComponentKey::Integer(key)) => {
                label.push_str(&format!("/key:{key}"));
            }
            ComponentSlot::Key(ComponentKey::Number(key)) => {
                label.push_str(&format!("/key:{}", f64::from_bits(*key)));
            }
            ComponentSlot::Key(ComponentKey::String(key)) => {
                label.push_str(&format!("/key:{key}"));
            }
        }
    }
    label
}

fn hook_signature_label(signature: HookSignature) -> String {
    let kind = match signature.kind {
        HookKind::State => "use_state",
        HookKind::Reducer => "use_reducer",
        HookKind::Ref => "use_ref",
        HookKind::Memo => "use_memo",
        HookKind::Callback => "use_callback",
        HookKind::Effect => "use_effect",
        HookKind::Store => "store.use_state",
        HookKind::Window => "use_window_open",
    };
    let mut label = match signature.value_type {
        Some(value_type) => {
            let value_type = match value_type {
                HookValueType::Nil => "nil",
                HookValueType::Boolean => "boolean",
                HookValueType::Number => "number",
                HookValueType::String => "string",
                HookValueType::Table => "table",
                HookValueType::Function => "function",
                HookValueType::Thread => "thread",
                HookValueType::UserData => "userdata",
                HookValueType::LightUserData => "lightuserdata",
                HookValueType::Error => "error",
                HookValueType::Other => "other",
            };
            format!("{kind}({value_type})")
        }
        None => kind.to_string(),
    };
    if let Some(site) = signature.site {
        label.push_str(&format!(" at LuaX site {site:016x}"));
    }
    label
}

fn hook_value_type(value: &Value) -> HookValueType {
    match value {
        Value::Nil => HookValueType::Nil,
        Value::Boolean(_) => HookValueType::Boolean,
        Value::Integer(_) | Value::Number(_) => HookValueType::Number,
        Value::String(_) => HookValueType::String,
        Value::Table(_) => HookValueType::Table,
        Value::Function(_) => HookValueType::Function,
        Value::Thread(_) => HookValueType::Thread,
        Value::UserData(_) => HookValueType::UserData,
        Value::LightUserData(_) => HookValueType::LightUserData,
        Value::Error(_) => HookValueType::Error,
        _ => HookValueType::Other,
    }
}

fn hook_dependencies(lua: &Lua, table: Option<Table>) -> mlua::Result<Option<DependencyList>> {
    let Some(table) = table else {
        return Ok(None);
    };
    let len = table.raw_len();
    let mut seen = 0usize;
    for pair in table.clone().pairs::<Value, Value>() {
        let (key, _) = pair?;
        let Value::Integer(index) = key else {
            return Err(mlua::Error::runtime(
                "Lua hook dependencies must be a dense array",
            ));
        };
        if index < 1 || index as usize > len {
            return Err(mlua::Error::runtime(
                "Lua hook dependencies must be a dense array",
            ));
        }
        seen += 1;
    }
    if seen != len {
        return Err(mlua::Error::runtime(
            "Lua hook dependencies must be a dense array",
        ));
    }
    let mut dependencies = Vec::with_capacity(len);
    for index in 1..=len {
        let value = table.raw_get::<Value>(index)?;
        dependencies.push(Arc::new(lua.create_registry_value(value)?));
    }
    Ok(Some(DependencyList(dependencies)))
}

fn hook_dependencies_equal(
    lua: &Lua,
    previous: &Option<DependencyList>,
    next: &Option<DependencyList>,
) -> mlua::Result<bool> {
    let (Some(previous), Some(next)) = (previous, next) else {
        return Ok(false);
    };
    if previous.0.len() != next.0.len() {
        return Ok(false);
    }
    for (previous, next) in previous.0.iter().zip(&next.0) {
        let previous = lua.registry_value::<Value>(previous.as_ref())?;
        let next = lua.registry_value::<Value>(next.as_ref())?;
        if previous != next {
            return Ok(false);
        }
    }
    Ok(true)
}

fn component_key(value: Value) -> mlua::Result<Option<ComponentKey>> {
    match value {
        Value::Nil => Ok(None),
        Value::Integer(value) => Ok(Some(ComponentKey::Integer(value))),
        Value::Number(value) if value.is_finite() => {
            Ok(Some(ComponentKey::Number(value.to_bits())))
        }
        Value::String(value) => Ok(Some(ComponentKey::String(value.to_str()?.to_string()))),
        value => Err(mlua::Error::runtime(format!(
            "Lua component keys must be strings or numbers, got {}",
            value.type_name()
        ))),
    }
}

fn select_store_value(
    lua: &Lua,
    selector: &Option<Arc<RegistryKey>>,
    state: Value,
) -> mlua::Result<Value> {
    match selector {
        Some(selector) => lua
            .registry_value::<Function>(selector.as_ref())?
            .call::<Value>(state),
        None => Ok(state),
    }
}

fn store_values_equal(
    lua: &Lua,
    equality: &Option<Arc<RegistryKey>>,
    previous: Value,
    next: Value,
) -> mlua::Result<bool> {
    match equality {
        Some(equality) => lua
            .registry_value::<Function>(equality.as_ref())?
            .call::<bool>((previous, next)),
        None => Ok(previous == next),
    }
}

fn prepare_store_updates(
    lua: &Lua,
    subscribers: Vec<StoreSubscriber>,
    next_state: &Value,
) -> mlua::Result<Vec<(HookId, Arc<RegistryKey>)>> {
    let mut updates = Vec::new();
    for subscriber in subscribers {
        let previous = lua.registry_value::<Value>(subscriber.selected.as_ref())?;
        let next = select_store_value(lua, &subscriber.selector, next_state.clone())?;
        if !store_values_equal(lua, &subscriber.equality, previous, next.clone())? {
            updates.push((subscriber.id, Arc::new(lua.create_registry_value(next)?)));
        }
    }
    Ok(updates)
}

fn clear_store_dispatch(stores: &Arc<Mutex<StoreRegistry>>, name: &str) {
    stores.lock().unwrap().dispatching.remove(name);
}

fn dispatch_store(
    lua: &Lua,
    name: &str,
    action: Value,
    stores: &Arc<Mutex<StoreRegistry>>,
    hooks: &Arc<Mutex<HookStore>>,
    memo: &Arc<Mutex<MemoStore>>,
) -> mlua::Result<Value> {
    let (state, reducer) = {
        let mut stores = stores.lock().unwrap();
        if !stores.dispatching.insert(name.to_string()) {
            return Err(mlua::Error::runtime(format!(
                "store {name:?} dispatched while its reducer was already running"
            )));
        }
        let Some(store) = stores.entries.get(name) else {
            stores.dispatching.remove(name);
            return Err(mlua::Error::runtime(format!(
                "store {name:?} no longer exists"
            )));
        };
        (store.state.clone(), store.reducer.clone())
    };

    let current = match lua.registry_value::<Value>(state.as_ref()) {
        Ok(current) => current,
        Err(error) => {
            clear_store_dispatch(stores, name);
            return Err(error);
        }
    };
    let reducer = match lua.registry_value::<Function>(reducer.as_ref()) {
        Ok(reducer) => reducer,
        Err(error) => {
            clear_store_dispatch(stores, name);
            return Err(error);
        }
    };
    let next = match reducer.call::<Value>((current, action.clone())) {
        Ok(next) => next,
        Err(error) => {
            clear_store_dispatch(stores, name);
            return Err(error);
        }
    };
    let subscribers = hooks.lock().unwrap().store_subscribers(name);
    let updates = match prepare_store_updates(lua, subscribers, &next) {
        Ok(updates) => updates,
        Err(error) => {
            clear_store_dispatch(stores, name);
            return Err(error);
        }
    };
    let next = match lua.create_registry_value(next) {
        Ok(next) => Arc::new(next),
        Err(error) => {
            clear_store_dispatch(stores, name);
            return Err(error);
        }
    };
    let listeners = {
        let mut stores = stores.lock().unwrap();
        let Some(store) = stores.entries.get_mut(name) else {
            stores.dispatching.remove(name);
            return Err(mlua::Error::runtime(format!(
                "store {name:?} no longer exists"
            )));
        };
        store.state = next;
        let listeners = store.listeners.values().cloned().collect::<Vec<_>>();
        stores.dispatching.remove(name);
        listeners
    };
    let changed_components = hooks.lock().unwrap().apply_store_updates(updates);
    if !changed_components.is_empty() {
        memo.lock().unwrap().entries.retain(|_, entry| {
            !entry
                .components
                .iter()
                .any(|component| changed_components.contains(component))
        });
    }
    for listener in listeners {
        let result = lua
            .registry_value::<Function>(listener.as_ref())
            .and_then(|listener| listener.call::<()>(()));
        if let Err(error) = result {
            eprintln!("gpuix-lua store listener error: {error}");
        }
    }
    Ok(action)
}

fn create_store_table(
    lua: &Lua,
    name: String,
    stores: Arc<Mutex<StoreRegistry>>,
    hooks: Arc<Mutex<HookStore>>,
    memo: Arc<Mutex<MemoStore>>,
) -> mlua::Result<Table> {
    let store = lua.create_table()?;
    store.set("name", name.as_str())?;

    let state_name = name.clone();
    let state_stores = stores.clone();
    store.set(
        "get_state",
        lua.create_function(move |lua, ()| {
            let state = state_stores
                .lock()
                .unwrap()
                .entries
                .get(&state_name)
                .map(|store| store.state.clone())
                .ok_or_else(|| {
                    mlua::Error::runtime(format!("store {state_name:?} no longer exists"))
                })?;
            lua.registry_value::<Value>(state.as_ref())
        })?,
    )?;

    let dispatch_name = name.clone();
    let dispatch_stores = stores.clone();
    let dispatch_hooks = hooks.clone();
    let dispatch_memo = memo.clone();
    store.set(
        "dispatch",
        lua.create_function(move |lua, action: Value| {
            dispatch_store(
                lua,
                &dispatch_name,
                action,
                &dispatch_stores,
                &dispatch_hooks,
                &dispatch_memo,
            )
        })?,
    )?;

    let subscribe_name = name.clone();
    let subscribe_stores = stores.clone();
    store.set(
        "subscribe",
        lua.create_function(move |lua, listener: Function| {
            let listener = Arc::new(lua.create_registry_value(listener)?);
            let id = {
                let mut stores = subscribe_stores.lock().unwrap();
                let store = stores.entries.get_mut(&subscribe_name).ok_or_else(|| {
                    mlua::Error::runtime(format!("store {subscribe_name:?} no longer exists"))
                })?;
                let id = store.next_listener_id;
                store.next_listener_id = store.next_listener_id.wrapping_add(1);
                store.listeners.insert(id, listener);
                id
            };
            let unsubscribe_name = subscribe_name.clone();
            let unsubscribe_stores = subscribe_stores.clone();
            lua.create_function(move |_, ()| {
                if let Some(store) = unsubscribe_stores
                    .lock()
                    .unwrap()
                    .entries
                    .get_mut(&unsubscribe_name)
                {
                    store.listeners.remove(&id);
                }
                Ok(())
            })
        })?,
    )?;

    let hook_name = name;
    let hook_stores = stores;
    let hook_hooks = hooks;
    store.set(
        "use_state",
        lua.create_function(move |lua, arguments: StoreHookArguments| {
            let HookArguments((selector, equality), site) = arguments;
            let state = hook_stores
                .lock()
                .unwrap()
                .entries
                .get(&hook_name)
                .map(|store| store.state.clone())
                .ok_or_else(|| {
                    mlua::Error::runtime(format!("store {hook_name:?} no longer exists"))
                })?;
            let state = lua.registry_value::<Value>(state.as_ref())?;
            let selector = selector
                .map(|selector| lua.create_registry_value(selector).map(Arc::new))
                .transpose()?;
            let equality = equality
                .map(|equality| lua.create_registry_value(equality).map(Arc::new))
                .transpose()?;
            let selected = select_store_value(lua, &selector, state)?;
            let signature = HookSignature {
                kind: HookKind::Store,
                value_type: None,
                site,
            };
            let (id, existing) = hook_hooks
                .lock()
                .unwrap()
                .next_hook(signature)
                .map_err(mlua::Error::runtime)?;
            if existing.is_some() && !matches!(&existing, Some(HookSlot::Store { .. })) {
                return Err(mlua::Error::runtime(
                    "store.use_state found an incompatible hook slot",
                ));
            }
            let slot = HookSlot::Store {
                store: hook_name.clone(),
                selector,
                equality,
                selected: Arc::new(lua.create_registry_value(selected.clone())?),
            };
            if existing.is_some() {
                hook_hooks
                    .lock()
                    .unwrap()
                    .replace_slot(&id, slot)
                    .map_err(mlua::Error::runtime)?;
            } else {
                hook_hooks.lock().unwrap().push_slot(&id, slot);
            }
            Ok(selected)
        })?,
    )?;

    Ok(store)
}

fn validate_app_handle(handle: &AnyUserData) -> mlua::Result<()> {
    handle.borrow::<LuaAppHandle>().map(|_| ()).map_err(|_| {
        mlua::Error::runtime("gpuix application function expects a value from gpuix.create_app")
    })
}

fn positive_window_dimension(value: Option<f32>, default: f32, name: &str) -> mlua::Result<f32> {
    let value = value.unwrap_or(default);
    if !value.is_finite() || value <= 0.0 {
        return Err(mlua::Error::runtime(format!(
            "window {name} must be greater than zero"
        )));
    }
    Ok(value)
}

fn parse_window_definition(table: Table) -> mlua::Result<LuaWindowDefinition> {
    let id = table.get::<String>("id")?;
    if id.trim().is_empty() {
        return Err(mlua::Error::runtime("window id cannot be empty"));
    }
    let render = table.get::<Function>("render")?;
    let title = table
        .get::<Option<String>>("title")?
        .unwrap_or_else(|| id.clone());
    let width = positive_window_dimension(table.get("width")?, 800.0, "width")?;
    let height = positive_window_dimension(table.get("height")?, 600.0, "height")?;
    let open = table.get::<Option<bool>>("open")?.unwrap_or(true);
    let focus = table.get::<Option<bool>>("focus")?.unwrap_or(true);
    let reposition = table.get::<Option<bool>>("reposition")?.unwrap_or(false);
    Ok(LuaWindowDefinition {
        options: LuaWindowOptions {
            id,
            title,
            width,
            height,
            open,
            focus,
            reposition,
        },
        render,
    })
}

fn queue_app_command(
    handle: AnyUserData,
    id: String,
    application: &Arc<Mutex<LuaApplicationRegistry>>,
    command: impl FnOnce(String) -> LuaAppCommand,
) -> mlua::Result<()> {
    validate_app_handle(&handle)?;
    let mut application = application.lock().unwrap();
    if !application.definition_exists(&id) {
        return Err(mlua::Error::runtime(format!(
            "Lua window {id:?} is not defined"
        )));
    }
    application.commands.push_back(command(id));
    Ok(())
}

fn install_api(
    lua: &Lua,
    hooks: Arc<Mutex<HookStore>>,
    stores: Arc<Mutex<StoreRegistry>>,
    memo: Arc<Mutex<MemoStore>>,
    arena: Arc<Mutex<RenderArena>>,
    host_handles: Arc<Mutex<MountedHostHandleMap>>,
    host_updates: HostUpdates,
    root_cleanups: RootCleanups,
    focus_request: Arc<Mutex<Option<LuaFocusRequest>>>,
    open_windows: Arc<Mutex<HashSet<String>>>,
    application: Arc<Mutex<LuaApplicationRegistry>>,
    styles: Arc<Mutex<StyleCache>>,
) -> mlua::Result<()> {
    let api = lua.create_table()?;

    let prop_updates = host_updates.clone();
    api.set(
        "set_prop",
        lua.create_function(move |lua, (handle, key, value): (i64, String, Value)| {
            if matches!(
                key.as_str(),
                "key"
                    | "children"
                    | "style"
                    | "content"
                    | "autoFocus"
                    | "testId"
                    | "ref"
                    | "className"
            ) || key.starts_with("on")
            {
                return Err(mlua::Error::runtime(
                    "set_prop accepts custom host props, not structural props or events",
                ));
            }
            let value = if matches!(value, Value::Nil) {
                serde_json::Value::Null
            } else {
                lua.from_value(value)?
            };
            prop_updates
                .lock()
                .unwrap()
                .push(HostUpdate::Prop(handle, key, value));
            Ok(())
        })?,
    )?;

    let child_updates = host_updates.clone();
    let child_arena = arena.clone();
    api.set(
        "set_child",
        lua.create_function(move |_, (parent, value): (i64, Value)| {
            let index = if matches!(value, Value::Nil) {
                None
            } else {
                let handle = node_handle(&value)?;
                let mut arena = child_arena.lock().unwrap();
                arena.validate_root(handle)?;
                arena.parented[handle.index] = true;
                Some(handle.index)
            };
            child_updates
                .lock()
                .unwrap()
                .push(HostUpdate::Child(parent, index));
            Ok(())
        })?,
    )?;

    let children_updates = host_updates.clone();
    api.set(
        "set_children",
        lua.create_function(move |_, (parent, children): (i64, Table)| {
            let length = children.raw_len();
            for entry in children.clone().pairs::<Value, Value>() {
                let (key, _) = entry?;
                if !matches!(key, Value::Integer(index) if index > 0 && index as usize <= length) {
                    return Err(mlua::Error::runtime("children must be a dense array"));
                }
            }
            let tokens = children
                .sequence_values::<i64>()
                .collect::<mlua::Result<Vec<_>>>()?;
            if tokens.len() != length {
                return Err(mlua::Error::runtime("children must be a dense array"));
            }
            children_updates
                .lock()
                .unwrap()
                .push(HostUpdate::Children(parent, tokens));
            Ok(())
        })?,
    )?;

    let cleanup_hooks = hooks.clone();
    api.set(
        "on_root_cleanup",
        lua.create_function(move |_, callback: Function| {
            let root = cleanup_hooks
                .lock()
                .unwrap()
                .current_root()
                .map_err(mlua::Error::runtime)?;
            root_cleanups
                .lock()
                .unwrap()
                .entry(root)
                .or_default()
                .push(callback);
            Ok(())
        })?,
    )?;

    let text_updates = host_updates.clone();
    api.set(
        "set_text",
        lua.create_function(move |_, (handle, value): (i64, Value)| {
            let content = parse_text_content(value)?
                .ok_or_else(|| mlua::Error::runtime("set_text requires text"))?;
            text_updates
                .lock()
                .unwrap()
                .push(HostUpdate::Text(handle, content));
            Ok(())
        })?,
    )?;
    let style_updates = host_updates;
    let update_styles = styles.clone();
    api.set(
        "set_style",
        lua.create_function(move |lua, (handle, value): (i64, Value)| {
            let style = parse_style(lua, value, &update_styles)?
                .unwrap_or_else(|| Arc::new(StyleDesc::default()));
            style_updates
                .lock()
                .unwrap()
                .push(HostUpdate::Style(handle, style));
            Ok(())
        })?,
    )?;

    let create_app_registry = application.clone();
    api.set(
        "create_app",
        lua.create_function(move |lua, ()| {
            let mut application = create_app_registry.lock().unwrap();
            if application.created {
                return Err(mlua::Error::runtime(
                    "gpuix.create_app may only be called once per entry",
                ));
            }
            application.created = true;
            lua.create_userdata(LuaAppHandle)
        })?,
    )?;

    let define_window_registry = application.clone();
    api.set(
        "define_window",
        lua.create_function(move |_, (handle, definition): (AnyUserData, Table)| {
            validate_app_handle(&handle)?;
            let definition = parse_window_definition(definition)?;
            let mut application = define_window_registry.lock().unwrap();
            if !application.created {
                return Err(mlua::Error::runtime(
                    "gpuix.define_window requires an active application",
                ));
            }
            if application.definition_exists(&definition.options.id) {
                return Err(mlua::Error::runtime(format!(
                    "duplicate Lua window id {:?}",
                    definition.options.id
                )));
            }
            application.definitions.push(definition);
            Ok(())
        })?,
    )?;

    let open_window_registry = application.clone();
    api.set(
        "open_window",
        lua.create_function(move |_, (handle, id): (AnyUserData, String)| {
            queue_app_command(handle, id, &open_window_registry, LuaAppCommand::Open)
        })?,
    )?;

    let close_window_registry = application.clone();
    api.set(
        "close_window",
        lua.create_function(move |_, (handle, id): (AnyUserData, String)| {
            queue_app_command(handle, id, &close_window_registry, LuaAppCommand::Close)
        })?,
    )?;

    let focus_window_registry = application.clone();
    api.set(
        "focus_window",
        lua.create_function(move |_, (handle, id): (AnyUserData, String)| {
            queue_app_command(handle, id, &focus_window_registry, LuaAppCommand::Focus)
        })?,
    )?;

    let title_window_registry = application.clone();
    api.set(
        "set_window_title",
        lua.create_function(
            move |_, (handle, id, title): (AnyUserData, String, String)| {
                validate_app_handle(&handle)?;
                let mut application = title_window_registry.lock().unwrap();
                if !application.definition_exists(&id) {
                    return Err(mlua::Error::runtime(format!(
                        "Lua window {id:?} is not defined"
                    )));
                }
                application
                    .commands
                    .push_back(LuaAppCommand::SetTitle { id, title });
                Ok(())
            },
        )?,
    )?;

    let window_hook_application = application.clone();
    let window_hook_windows = open_windows.clone();
    let window_hook_hooks = hooks.clone();
    api.set(
        "use_window_open",
        lua.create_function(move |_, arguments: WindowHookArguments| {
            let HookArguments((handle, window_id), site) = arguments;
            validate_app_handle(&handle)?;
            if !window_hook_application
                .lock()
                .unwrap()
                .definition_exists(&window_id)
            {
                return Err(mlua::Error::runtime(format!(
                    "Lua window {window_id:?} is not defined"
                )));
            }
            let open = window_hook_windows.lock().unwrap().contains(&window_id);
            let signature = HookSignature {
                kind: HookKind::Window,
                value_type: None,
                site,
            };
            let (hook_id, existing) = window_hook_hooks
                .lock()
                .unwrap()
                .next_hook(signature)
                .map_err(mlua::Error::runtime)?;
            if existing.is_some() && !matches!(&existing, Some(HookSlot::Window { .. })) {
                return Err(mlua::Error::runtime(
                    "gpuix.use_window_open found an incompatible hook slot",
                ));
            }
            let slot = HookSlot::Window {
                id: window_id,
                open,
            };
            if existing.is_some() {
                window_hook_hooks
                    .lock()
                    .unwrap()
                    .replace_slot(&hook_id, slot)
                    .map_err(mlua::Error::runtime)?;
            } else {
                window_hook_hooks.lock().unwrap().push_slot(&hook_id, slot);
            }
            Ok(open)
        })?,
    )?;

    let h_arena = arena.clone();
    let h_styles = styles.clone();
    let h_hooks = hooks.clone();
    let h = lua.create_function(
        move |lua, (kind, props): (Value, Option<Table>)| match kind {
            Value::String(kind) => {
                create_host_node(lua, kind.to_str()?.as_ref(), props, &h_arena, &h_styles)
                    .map(Value::Integer)
            }
            Value::Function(component) => {
                let props = props.unwrap_or(lua.create_table()?);
                let key = component_key(props.raw_get::<Value>("key")?)?;
                h_hooks
                    .lock()
                    .unwrap()
                    .begin_component(key)
                    .map_err(mlua::Error::runtime)?;
                let value = match component.call::<Value>(props) {
                    Ok(value) => value,
                    Err(error) => {
                        h_hooks.lock().unwrap().abort_component();
                        return Err(error);
                    }
                };
                h_hooks
                    .lock()
                    .unwrap()
                    .finish_component()
                    .map_err(mlua::Error::runtime)?;
                Ok(value)
            }
            _ => Err(mlua::Error::runtime(
                "gpuix.h expects an element name or component function",
            )),
        },
    )?;
    api.set("h", h)?;

    let text_arena = arena.clone();
    let text_styles = styles.clone();
    let text = lua.create_function(move |lua, value: Value| match value {
        Value::Table(props) => {
            create_host_node(lua, "text", Some(props), &text_arena, &text_styles)
        }
        value => create_text_node(value, &text_arena),
    })?;
    api.set("text", text)?;

    let focus = lua.create_function(move |_, handle: i64| {
        let handle = host_handles
            .lock()
            .unwrap()
            .get(&handle)
            .cloned()
            .ok_or_else(|| mlua::Error::runtime("gpuix.focus expects a mounted host handle"))?;
        *focus_request.lock().unwrap() = Some(LuaFocusRequest {
            root_id: handle.root.to_string(),
            element_id: handle.element_id,
        });
        Ok(())
    })?;
    api.set("focus", focus)?;

    for (name, element_type) in ELEMENT_HELPERS {
        let element_type = (*element_type).to_string();
        let helper_arena = arena.clone();
        let helper_styles = styles.clone();
        let helper = lua.create_function(move |lua, props: Option<Table>| {
            create_host_node(lua, &element_type, props, &helper_arena, &helper_styles)
        })?;
        api.set(*name, helper)?;
    }

    let style_cache = styles.clone();
    let style = lua.create_function(move |lua, value: Value| {
        let style = parse_style(lua, value, &style_cache)?
            .ok_or_else(|| mlua::Error::runtime("gpuix.style expects a style table"))?;
        lua.create_userdata(StyleHandle(style))
    })?;
    api.set("style", style)?;

    let create_store_stores = stores.clone();
    let create_store_hooks = hooks.clone();
    let create_store_memo = memo.clone();
    let create_store = lua.create_function(
        move |lua, (name, reducer, initial_state): (String, Function, Value)| {
            if name.is_empty() {
                return Err(mlua::Error::runtime("gpuix.create_store name is empty"));
            }
            let reducer = Arc::new(lua.create_registry_value(reducer)?);
            let initial_state = Arc::new(lua.create_registry_value(initial_state)?);
            {
                let mut stores = create_store_stores.lock().unwrap();
                if let Some(store) = stores.entries.get_mut(&name) {
                    store.reducer = reducer;
                } else {
                    stores.entries.insert(
                        name.clone(),
                        StoreEntry {
                            state: initial_state,
                            reducer,
                            listeners: HashMap::new(),
                            next_listener_id: 1,
                        },
                    );
                }
            }
            create_store_table(
                lua,
                name,
                create_store_stores.clone(),
                create_store_hooks.clone(),
                create_store_memo.clone(),
            )
        },
    )?;
    api.set("create_store", create_store)?;

    let state_hooks = hooks.clone();
    let state_memo = memo.clone();
    let use_state = lua.create_function(move |lua, arguments: StateHookArguments| {
        let HookArguments(initial, site) = arguments;
        let signature = HookSignature {
            kind: HookKind::State,
            value_type: Some(hook_value_type(&initial)),
            site,
        };
        let (id, existing) = state_hooks
            .lock()
            .unwrap()
            .next_hook(signature)
            .map_err(mlua::Error::runtime)?;
        let (slot, setter) = match existing {
            Some(HookSlot::State { value, setter }) => (value, setter),
            Some(_) => {
                return Err(mlua::Error::runtime(
                    "gpuix.use_state found an incompatible hook slot",
                ));
            }
            None => {
                let slot = Arc::new(lua.create_registry_value(initial)?);
                let setter_hooks = state_hooks.clone();
                let setter_memo = state_memo.clone();
                let setter_id = id.clone();
                let setter = lua.create_function(move |lua, next: Value| {
                    let Some(HookSlot::State {
                        value: current_slot,
                        ..
                    }) = setter_hooks.lock().unwrap().slot(&setter_id)
                    else {
                        return Err(mlua::Error::runtime(
                            "Lua state setter refers to an unmounted hook",
                        ));
                    };
                    let current: Value = lua.registry_value(current_slot.as_ref())?;
                    let next = match next {
                        Value::Function(update) => update.call::<Value>(current)?,
                        value => value,
                    };
                    setter_hooks
                        .lock()
                        .unwrap()
                        .set_state(&setter_id, Arc::new(lua.create_registry_value(next)?))
                        .map_err(mlua::Error::runtime)?;
                    setter_memo
                        .lock()
                        .unwrap()
                        .entries
                        .retain(|_, entry| !entry.components.contains(&setter_id.component));
                    Ok(())
                })?;
                let setter = Arc::new(lua.create_registry_value(setter)?);
                state_hooks.lock().unwrap().push_slot(
                    &id,
                    HookSlot::State {
                        value: slot.clone(),
                        setter: setter.clone(),
                    },
                );
                (slot, setter)
            }
        };
        let value: Value = lua.registry_value(slot.as_ref())?;
        let setter = lua.registry_value::<Function>(setter.as_ref())?;
        Ok((value, setter))
    })?;
    api.set("use_state", use_state)?;

    let reducer_hooks = hooks.clone();
    let reducer_memo = memo.clone();
    let use_reducer = lua.create_function(move |lua, arguments: ReducerHookArguments| {
        let HookArguments((reducer, initial), site) = arguments;
        let signature = HookSignature {
            kind: HookKind::Reducer,
            value_type: Some(hook_value_type(&initial)),
            site,
        };
        let (id, existing) = reducer_hooks
            .lock()
            .unwrap()
            .next_hook(signature)
            .map_err(mlua::Error::runtime)?;
        let reducer_key = Arc::new(lua.create_registry_value(reducer)?);
        let (state, dispatch) = match existing {
            Some(HookSlot::Reducer {
                state, dispatch, ..
            }) => {
                reducer_hooks
                    .lock()
                    .unwrap()
                    .replace_slot(
                        &id,
                        HookSlot::Reducer {
                            state: state.clone(),
                            reducer: reducer_key,
                            dispatch: dispatch.clone(),
                        },
                    )
                    .map_err(mlua::Error::runtime)?;
                (state, dispatch)
            }
            Some(_) => {
                return Err(mlua::Error::runtime(
                    "gpuix.use_reducer found an incompatible hook slot",
                ));
            }
            None => {
                let state = Arc::new(lua.create_registry_value(initial)?);
                let dispatch_hooks = reducer_hooks.clone();
                let dispatch_memo = reducer_memo.clone();
                let dispatch_id = id.clone();
                let dispatch = lua.create_function(move |lua, action: Value| {
                    let Some(HookSlot::Reducer { state, reducer, .. }) =
                        dispatch_hooks.lock().unwrap().slot(&dispatch_id)
                    else {
                        return Err(mlua::Error::runtime(
                            "Lua reducer dispatch refers to an unmounted hook",
                        ));
                    };
                    let current = lua.registry_value::<Value>(state.as_ref())?;
                    let reducer = lua.registry_value::<Function>(reducer.as_ref())?;
                    let next = reducer.call::<Value>((current, action))?;
                    dispatch_hooks
                        .lock()
                        .unwrap()
                        .set_state(&dispatch_id, Arc::new(lua.create_registry_value(next)?))
                        .map_err(mlua::Error::runtime)?;
                    dispatch_memo
                        .lock()
                        .unwrap()
                        .entries
                        .retain(|_, entry| !entry.components.contains(&dispatch_id.component));
                    Ok(())
                })?;
                let dispatch = Arc::new(lua.create_registry_value(dispatch)?);
                reducer_hooks.lock().unwrap().push_slot(
                    &id,
                    HookSlot::Reducer {
                        state: state.clone(),
                        reducer: reducer_key,
                        dispatch: dispatch.clone(),
                    },
                );
                (state, dispatch)
            }
        };
        let value = lua.registry_value::<Value>(state.as_ref())?;
        let dispatch = lua.registry_value::<Function>(dispatch.as_ref())?;
        Ok((value, dispatch))
    })?;
    api.set("use_reducer", use_reducer)?;

    let ref_hooks = hooks.clone();
    let use_ref = lua.create_function(move |lua, arguments: StateHookArguments| {
        let HookArguments(initial, site) = arguments;
        let signature = HookSignature {
            kind: HookKind::Ref,
            value_type: Some(hook_value_type(&initial)),
            site,
        };
        let (id, existing) = ref_hooks
            .lock()
            .unwrap()
            .next_hook(signature)
            .map_err(mlua::Error::runtime)?;
        let slot = match existing {
            Some(HookSlot::Ref(slot)) => slot,
            Some(_) => {
                return Err(mlua::Error::runtime(
                    "gpuix.use_ref found an incompatible hook slot",
                ));
            }
            None => {
                let value = lua.create_table()?;
                value.set("current", initial)?;
                let slot = Arc::new(lua.create_registry_value(value)?);
                ref_hooks
                    .lock()
                    .unwrap()
                    .push_slot(&id, HookSlot::Ref(slot.clone()));
                slot
            }
        };
        lua.registry_value::<Table>(slot.as_ref())
    })?;
    api.set("use_ref", use_ref)?;

    let memo_hooks = hooks.clone();
    let use_memo = lua.create_function(move |lua, arguments: DependencyHookArguments| {
        let HookArguments((factory, dependencies), site) = arguments;
        let dependencies = hook_dependencies(lua, dependencies)?;
        let signature = HookSignature {
            kind: HookKind::Memo,
            value_type: None,
            site,
        };
        let (id, existing) = memo_hooks
            .lock()
            .unwrap()
            .next_hook(signature)
            .map_err(mlua::Error::runtime)?;
        let refreshing = memo_hooks.lock().unwrap().refreshing();
        if let Some(HookSlot::Memo {
            value,
            dependencies: previous,
        }) = existing
        {
            if !refreshing && hook_dependencies_equal(lua, &previous, &dependencies)? {
                return lua.registry_value::<Value>(value.as_ref());
            }
        }
        let value = factory.call::<Value>(())?;
        let slot = Arc::new(lua.create_registry_value(value.clone())?);
        let next = HookSlot::Memo {
            value: slot,
            dependencies,
        };
        if memo_hooks.lock().unwrap().slot(&id).is_some() {
            memo_hooks
                .lock()
                .unwrap()
                .replace_slot(&id, next)
                .map_err(mlua::Error::runtime)?;
        } else {
            memo_hooks.lock().unwrap().push_slot(&id, next);
        }
        Ok(value)
    })?;
    api.set("use_memo", use_memo)?;

    let callback_hooks = hooks.clone();
    let use_callback = lua.create_function(move |lua, arguments: DependencyHookArguments| {
        let HookArguments((callback, dependencies), site) = arguments;
        let dependencies = hook_dependencies(lua, dependencies)?;
        let signature = HookSignature {
            kind: HookKind::Callback,
            value_type: None,
            site,
        };
        let (id, existing) = callback_hooks
            .lock()
            .unwrap()
            .next_hook(signature)
            .map_err(mlua::Error::runtime)?;
        let refreshing = callback_hooks.lock().unwrap().refreshing();
        if let Some(HookSlot::Memo {
            value,
            dependencies: previous,
        }) = existing
        {
            if !refreshing && hook_dependencies_equal(lua, &previous, &dependencies)? {
                return lua.registry_value::<Function>(value.as_ref());
            }
        }
        let value = Arc::new(lua.create_registry_value(callback.clone())?);
        let next = HookSlot::Memo {
            value,
            dependencies,
        };
        if callback_hooks.lock().unwrap().slot(&id).is_some() {
            callback_hooks
                .lock()
                .unwrap()
                .replace_slot(&id, next)
                .map_err(mlua::Error::runtime)?;
        } else {
            callback_hooks.lock().unwrap().push_slot(&id, next);
        }
        Ok(callback)
    })?;
    api.set("use_callback", use_callback)?;

    let effect_hooks = hooks.clone();
    let use_effect = lua.create_function(move |lua, arguments: DependencyHookArguments| {
        let HookArguments((callback, dependencies), site) = arguments;
        let dependencies = hook_dependencies(lua, dependencies)?;
        let signature = HookSignature {
            kind: HookKind::Effect,
            value_type: None,
            site,
        };
        let (id, existing) = effect_hooks
            .lock()
            .unwrap()
            .next_hook(signature)
            .map_err(mlua::Error::runtime)?;
        let refreshing = effect_hooks.lock().unwrap().refreshing();
        let changed = match &existing {
            Some(HookSlot::Effect {
                dependencies: previous,
                ..
            }) => refreshing || !hook_dependencies_equal(lua, previous, &dependencies)?,
            Some(_) => {
                return Err(mlua::Error::runtime(
                    "gpuix.use_effect found an incompatible hook slot",
                ));
            }
            None => true,
        };
        if existing.is_none() {
            effect_hooks.lock().unwrap().push_slot(
                &id,
                HookSlot::Effect {
                    dependencies: None,
                    cleanup: None,
                },
            );
        }
        if changed {
            effect_hooks.lock().unwrap().queue_effect(
                id,
                Arc::new(lua.create_registry_value(callback)?),
                dependencies,
            );
        }
        Ok(())
    })?;
    api.set("use_effect", use_effect)?;

    let memo_arena = arena.clone();
    let memo_store = memo.clone();
    let memo_hooks = hooks.clone();
    let memo_fn = lua.create_function(
        move |_, (key, dependencies, render): (Value, Value, Function)| {
            let key = memo_key(&key)?;
            let dependencies = memo_dependency(&dependencies)?;
            let root = memo_hooks
                .lock()
                .unwrap()
                .current_root()
                .map_err(mlua::Error::runtime)?;
            if let Some(cached) = memo_lookup(&memo_store, root.clone(), &key, &dependencies)? {
                if !cached.component_slots.is_empty() {
                    memo_hooks
                        .lock()
                        .unwrap()
                        .replay_component_metadata(&cached.component_slots, &cached.components)
                        .map_err(mlua::Error::runtime)?;
                } else if !cached.components.is_empty() {
                    memo_hooks
                        .lock()
                        .unwrap()
                        .replay_component_metadata(&[], &cached.components)
                        .map_err(mlua::Error::runtime)?;
                }
                let handle = memo_arena.lock().unwrap().push_cached(cached.node);
                return handle_token(handle).map(Value::Integer);
            }

            let checkpoint = memo_hooks
                .lock()
                .unwrap()
                .component_checkpoint()
                .map_err(mlua::Error::runtime)?;
            let value = render.call::<Value>(())?;
            let (component_slots, components) = memo_hooks
                .lock()
                .unwrap()
                .component_metadata_since(checkpoint)
                .map_err(mlua::Error::runtime)?;
            memo_commit(
                &memo_store,
                &memo_arena,
                root,
                key,
                dependencies,
                component_slots,
                components,
                &value,
            )?;
            Ok(value)
        },
    )?;
    api.set("memo", memo_fn)?;

    let batch_arena = arena.clone();
    let batch_hooks = hooks.clone();
    let memo_batch = lua.create_function(
        move |_, (keys, dependencies, render): (Table, Table, Function)| {
            let len = keys.raw_len();
            if dependencies.raw_len() != len {
                return Err(mlua::Error::runtime(
                    "gpuix.memo_batch keys and dependencies must have equal lengths",
                ));
            }

            let mut batch_dependencies = Vec::with_capacity(len);
            dependencies.for_each_value::<Value>(|value| {
                let dependency = memo_dependency(&value)?;
                batch_dependencies.push((value, dependency));
                Ok(())
            })?;
            if batch_dependencies.len() != len {
                return Err(mlua::Error::runtime(
                    "gpuix.memo_batch keys and dependencies must be dense tables",
                ));
            }

            let mut children = Vec::with_capacity(len);
            keys.for_each_value::<Value>(|raw_key| {
                let offset = children.len();
                let Some((raw_dependencies, dependency)) = batch_dependencies.get(offset) else {
                    return Err(mlua::Error::runtime(
                        "gpuix.memo_batch keys and dependencies must be dense tables",
                    ));
                };
                let key = memo_key(&raw_key)?;
                let root = batch_hooks
                    .lock()
                    .unwrap()
                    .current_root()
                    .map_err(mlua::Error::runtime)?;
                let handle = if let Some(cached) =
                    memo_lookup(&memo, root.clone(), &key, &dependency)?
                {
                    if !cached.component_slots.is_empty() {
                        batch_hooks
                            .lock()
                            .unwrap()
                            .replay_component_metadata(&cached.component_slots, &cached.components)
                            .map_err(mlua::Error::runtime)?;
                    } else if !cached.components.is_empty() {
                        batch_hooks
                            .lock()
                            .unwrap()
                            .replay_component_metadata(&[], &cached.components)
                            .map_err(mlua::Error::runtime)?;
                    }
                    batch_arena.lock().unwrap().push_cached(cached.node)
                } else {
                    let checkpoint = batch_hooks
                        .lock()
                        .unwrap()
                        .component_checkpoint()
                        .map_err(mlua::Error::runtime)?;
                    let value =
                        render.call::<Value>((raw_key, raw_dependencies.clone(), offset + 1))?;
                    let (component_slots, components) = batch_hooks
                        .lock()
                        .unwrap()
                        .component_metadata_since(checkpoint)
                        .map_err(mlua::Error::runtime)?;
                    memo_commit(
                        &memo,
                        &batch_arena,
                        root,
                        key,
                        dependency.clone(),
                        component_slots,
                        components,
                        &value,
                    )?
                };
                children.push(handle.index);
                Ok(())
            })?;
            if children.len() != len {
                return Err(mlua::Error::runtime(
                    "gpuix.memo_batch keys and dependencies must be dense tables",
                ));
            }
            let handle = batch_arena.lock().unwrap().push_child_list(children);
            child_list_token(handle)
        },
    )?;
    api.set("memo_batch", memo_batch)?;
    lua.globals().set("gpuix", api)
}

fn memo_lookup(
    memo: &Arc<Mutex<MemoStore>>,
    root: LuaRootId,
    key: &MemoKey,
    dependencies: &MemoDependency,
) -> mlua::Result<Option<MemoHit>> {
    let mut memo = memo.lock().unwrap();
    let scoped_key = ScopedMemoKey {
        root,
        key: key.clone(),
    };
    if !memo.seen.insert(scoped_key.clone()) {
        return Err(mlua::Error::runtime(format!(
            "duplicate gpuix.memo key {key:?}"
        )));
    }
    Ok(memo
        .entries
        .get(&scoped_key)
        .filter(|entry| entry.dependencies == *dependencies)
        .map(|entry| MemoHit {
            node: CachedNode {
                memo_key: key.clone(),
                element_type: entry.element_type.clone(),
                key: entry.key.clone(),
            },
            component_slots: entry.component_slots.clone(),
            components: entry.components.clone(),
        }))
}

fn memo_commit(
    memo: &Arc<Mutex<MemoStore>>,
    arena: &Arc<Mutex<RenderArena>>,
    root: LuaRootId,
    key: MemoKey,
    dependencies: MemoDependency,
    component_slots: Vec<ComponentSlot>,
    components: Vec<ComponentId>,
    value: &Value,
) -> mlua::Result<NodeHandle> {
    let handle = node_handle(value)?;
    let (element_type, node_key) = {
        let mut arena = arena.lock().unwrap();
        arena.validate_handle(handle)?;
        let identity = arena.identity(handle.index).map_err(mlua::Error::runtime)?;
        arena.set_memo_key(handle, key.clone())?;
        identity
    };
    memo.lock().unwrap().entries.insert(
        ScopedMemoKey { root, key },
        MemoEntry {
            dependencies,
            element_type: Arc::from(element_type),
            key: node_key,
            component_slots: component_slots.into(),
            components: components.into(),
        },
    );
    Ok(handle)
}

fn create_host_node(
    lua: &Lua,
    element_type: &str,
    props: Option<Table>,
    arena: &Arc<Mutex<RenderArena>>,
    styles: &Arc<Mutex<StyleCache>>,
) -> mlua::Result<i64> {
    let mut key = None;
    let mut style = None;
    let mut content = None;
    let mut events = HashMap::new();
    let mut custom_props = HashMap::new();
    let mut auto_focus = false;
    let mut test_id = None;
    let mut indexed_children = Vec::new();
    let mut named_children = None;
    let built_in = element_type == "div" || element_type == "text";

    if let Some(props) = props {
        for pair in props.pairs::<Value, Value>() {
            let (prop, value) = pair?;
            match prop {
                Value::Integer(index) if index > 0 => indexed_children.push((index, value)),
                Value::String(prop) => {
                    let prop = prop.to_str()?;
                    match prop.as_ref() {
                        "key" => key = parse_key(value)?,
                        "style" => style = parse_style(lua, value, styles)?,
                        "content" if element_type == "text" => content = parse_text_content(value)?,
                        "children" => named_children = Some(value),
                        "autoFocus" => auto_focus = matches!(value, Value::Boolean(true)),
                        "testId" => {
                            if let Value::String(value) = value {
                                test_id = Some(value.to_str()?.to_string());
                            }
                        }
                        "className" | "ref" => {}
                        name => {
                            if let Some(event_type) = event_type(name) {
                                let Value::Function(handler) = value else {
                                    return Err(mlua::Error::runtime(format!(
                                        "{name} must be a function"
                                    )));
                                };
                                events.insert(event_type.to_string(), handler);
                            } else if !built_in || is_universal_prop(name) {
                                if !matches!(value, Value::Nil | Value::Function(_)) {
                                    custom_props.insert(name.to_string(), lua.from_value(value)?);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    indexed_children.sort_by_key(|(index, _)| *index);
    let mut children = Vec::new();
    for (_, value) in indexed_children {
        append_children(value, arena, &mut children)?;
    }
    if let Some(value) = named_children {
        append_children(value, arena, &mut children)?;
    }
    if content.is_some() && !children.is_empty() {
        return Err(mlua::Error::runtime(
            "gpuix.text cannot have both content and child handles",
        ));
    }

    let handle = arena.lock().unwrap().push_pending(PendingNode {
        element_type: element_type.to_string(),
        key,
        style,
        content,
        events,
        custom_props,
        auto_focus,
        test_id,
        children,
        memo_key: None,
    })?;
    handle_token(handle)
}

fn append_children(
    value: Value,
    arena: &Arc<Mutex<RenderArena>>,
    children: &mut Vec<usize>,
) -> mlua::Result<()> {
    match value {
        Value::Nil | Value::Boolean(false) => Ok(()),
        Value::Integer(value) => {
            if value < 0 {
                let handle = child_list_handle(value)?;
                let mut child_list = arena.lock().unwrap().take_child_list(handle)?;
                if children.is_empty() {
                    *children = child_list;
                } else {
                    children.append(&mut child_list);
                }
            } else {
                let handle = node_handle(&Value::Integer(value))?;
                arena.lock().unwrap().validate_handle(handle)?;
                children.push(handle.index);
            }
            Ok(())
        }
        Value::Table(values) => {
            for value in values.sequence_values::<Value>() {
                append_children(value?, arena, children)?;
            }
            Ok(())
        }
        other => Err(mlua::Error::runtime(format!(
            "Lua children must be host handles, not {}; wrap text with gpuix.text(value)",
            other.type_name()
        ))),
    }
}

fn create_text_node(value: Value, arena: &Arc<Mutex<RenderArena>>) -> mlua::Result<i64> {
    let content = parse_text_content(value)?.ok_or_else(|| {
        mlua::Error::runtime("gpuix.text expects a string, number, or props table")
    })?;
    let handle = arena.lock().unwrap().push_pending(PendingNode {
        element_type: "text".to_string(),
        key: None,
        style: None,
        content: Some(content),
        events: HashMap::new(),
        custom_props: HashMap::new(),
        auto_focus: false,
        test_id: None,
        children: Vec::new(),
        memo_key: None,
    })?;
    handle_token(handle)
}

fn parse_text_content(value: Value) -> mlua::Result<Option<String>> {
    match value {
        Value::Nil | Value::Boolean(false) => Ok(None),
        Value::String(value) => Ok(Some(value.to_str()?.to_string())),
        Value::Integer(value) => Ok(Some(value.to_string())),
        Value::Number(value) => Ok(Some(value.to_string())),
        other => Err(mlua::Error::runtime(format!(
            "unsupported Lua text value: {}",
            other.type_name()
        ))),
    }
}

fn parse_style(
    lua: &Lua,
    value: Value,
    styles: &Arc<Mutex<StyleCache>>,
) -> mlua::Result<Option<Arc<StyleDesc>>> {
    match value {
        Value::Nil => Ok(None),
        Value::UserData(value) => Ok(Some(value.borrow::<StyleHandle>()?.0.clone())),
        value => {
            let value: serde_json::Value = lua.from_value(value)?;
            let key = serde_json::to_vec(&value).map_err(mlua::Error::external)?;
            if let Some(style) = styles.lock().unwrap().get(&key).cloned() {
                return Ok(Some(style));
            }
            let style = Arc::new(
                serde_json::from_value::<StyleDesc>(value).map_err(mlua::Error::external)?,
            );
            let mut styles = styles.lock().unwrap();
            if styles.len() >= 1024 {
                styles.clear();
            }
            styles.insert(key, style.clone());
            Ok(Some(style))
        }
    }
}

fn parse_key(value: Value) -> mlua::Result<Option<Arc<str>>> {
    match value {
        Value::Nil => Ok(None),
        Value::String(value) => Ok(Some(Arc::from(value.to_str()?.as_ref()))),
        Value::Integer(value) => Ok(Some(Arc::from(value.to_string()))),
        Value::Number(value) => Ok(Some(Arc::from(value.to_string()))),
        _ => Err(mlua::Error::runtime(
            "Lua element key must be a string or number",
        )),
    }
}

fn memo_key(value: &Value) -> mlua::Result<MemoKey> {
    match value {
        Value::Integer(value) => Ok(MemoKey::Integer(*value)),
        Value::Number(value) => Ok(match exact_integer(*value) {
            Some(value) => MemoKey::Integer(value),
            None => MemoKey::Number(normalized_number_bits(*value)),
        }),
        Value::String(value) => Ok(MemoKey::String(value.to_str()?.to_string())),
        Value::Nil => Err(mlua::Error::runtime("gpuix.memo key cannot be nil")),
        other => Err(mlua::Error::runtime(format!(
            "gpuix.memo key must be a string or number, got {}",
            other.type_name()
        ))),
    }
}

fn memo_dependency(value: &Value) -> mlua::Result<MemoDependency> {
    memo_dependency_inner(value, &mut HashSet::new())
}

fn memo_dependency_inner(
    value: &Value,
    visiting: &mut HashSet<usize>,
) -> mlua::Result<MemoDependency> {
    Ok(match value {
        Value::Nil => MemoDependency::Nil,
        Value::Boolean(value) => MemoDependency::Boolean(*value),
        Value::Integer(value) => MemoDependency::Integer(*value),
        Value::Number(value) => match exact_integer(*value) {
            Some(value) => MemoDependency::Integer(value),
            None => MemoDependency::Number(normalized_number_bits(*value)),
        },
        Value::String(value) => MemoDependency::String(value.to_str()?.to_string()),
        Value::Table(table) => {
            let pointer = table.to_pointer() as usize;
            if !visiting.insert(pointer) {
                return Err(mlua::Error::runtime(
                    "gpuix.memo dependencies cannot contain cyclic tables",
                ));
            }
            let result: mlua::Result<MemoDependency> = (|| {
                let mut entries = Vec::with_capacity(table.raw_len());
                for pair in table.pairs::<Value, Value>() {
                    let (key, value) = pair?;
                    entries.push((
                        memo_dependency_key(&key)?,
                        memo_dependency_inner(&value, visiting)?,
                    ));
                }
                entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
                Ok(MemoDependency::Table(entries))
            })();
            visiting.remove(&pointer);
            result?
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "unsupported gpuix.memo dependency type: {}",
                other.type_name()
            )));
        }
    })
}

fn memo_dependency_key(value: &Value) -> mlua::Result<MemoDependencyKey> {
    match value {
        Value::Boolean(value) => Ok(MemoDependencyKey::Boolean(*value)),
        Value::Integer(value) => Ok(MemoDependencyKey::Integer(*value)),
        Value::Number(value) => Ok(match exact_integer(*value) {
            Some(value) => MemoDependencyKey::Integer(value),
            None => MemoDependencyKey::Number(normalized_number_bits(*value)),
        }),
        Value::String(value) => Ok(MemoDependencyKey::String(value.to_str()?.to_string())),
        other => Err(mlua::Error::runtime(format!(
            "unsupported gpuix.memo dependency table key type: {}",
            other.type_name()
        ))),
    }
}

fn exact_integer(value: f64) -> Option<i64> {
    const MAX_EXACT_INTEGER: f64 = 9_007_199_254_740_992.0;
    (value.is_finite() && value.fract() == 0.0 && value.abs() <= MAX_EXACT_INTEGER)
        .then_some(value as i64)
}

fn normalized_number_bits(value: f64) -> u64 {
    if value == 0.0 {
        0.0f64.to_bits()
    } else if value.is_nan() {
        f64::NAN.to_bits()
    } else {
        value.to_bits()
    }
}

fn handle_token(handle: NodeHandle) -> mlua::Result<i64> {
    packed_handle_token(handle.generation, handle.index, "nodes")
}

fn child_list_token(handle: ChildListHandle) -> mlua::Result<i64> {
    Ok(-packed_handle_token(
        handle.generation,
        handle.index,
        "child lists",
    )?)
}

fn packed_handle_token(generation: u64, index: usize, kind: &str) -> mlua::Result<i64> {
    let index = u32::try_from(index)
        .map_err(|_| mlua::Error::runtime(format!("Lua render arena exceeded 2^32 {kind}")))?;
    Ok(((generation << HANDLE_INDEX_BITS) | u64::from(index)) as i64)
}

fn node_handle(value: &Value) -> mlua::Result<NodeHandle> {
    let Value::Integer(value) = value else {
        return Err(mlua::Error::runtime(
            "Lua render function must return one host handle",
        ));
    };
    if *value <= 0 {
        return Err(mlua::Error::runtime("Lua host handle is invalid"));
    }
    let value = *value as u64;
    Ok(NodeHandle {
        generation: value >> HANDLE_INDEX_BITS,
        index: (value & HANDLE_INDEX_MASK) as usize,
    })
}

fn child_list_handle(value: i64) -> mlua::Result<ChildListHandle> {
    let value = value
        .checked_neg()
        .filter(|value| *value > 0)
        .ok_or_else(|| mlua::Error::runtime("Lua child list handle is invalid"))?
        as u64;
    Ok(ChildListHandle {
        generation: value >> HANDLE_INDEX_BITS,
        index: (value & HANDLE_INDEX_MASK) as usize,
    })
}

fn event_type(prop: &str) -> Option<&'static str> {
    Some(match prop {
        "onToggleFile" => "toggleFile",
        "onShowMore" => "showMore",
        "onLineClick" => "lineClick",
        "onLinkClick" => "linkClick",
        "onVisibleRange" => "visibleRange",
        "onHighlight" => "highlight",
        "onChange" => "change",
        "onSubmit" => "submit",
        "onClick" => "click",
        "onAuxClick" => "auxClick",
        "onMouseDown" => "mouseDown",
        "onMouseUp" => "mouseUp",
        "onMouseEnter" => "mouseEnter",
        "onMouseLeave" => "mouseLeave",
        "onMouseMove" => "mouseMove",
        "onMouseDownOutside" => "mouseDownOutside",
        "onKeyDown" => "keyDown",
        "onKeyUp" => "keyUp",
        "onFocus" => "focus",
        "onBlur" => "blur",
        "onScroll" => "scroll",
        _ => return None,
    })
}

fn is_universal_prop(prop: &str) -> bool {
    matches!(prop, "tabIndex" | "motion" | "highlight")
}

fn reconcile_node(
    tree: &mut RetainedTree,
    next_id: &mut u64,
    old: Option<LuaNode>,
    arena: &mut RenderArena,
    index: usize,
    handle_aliases: &mut HostHandleMap,
) -> Result<LuaNode, String> {
    let handle_token =
        packed_handle_token(arena.generation, index, "nodes").map_err(|error| error.to_string())?;
    match arena.take(index)? {
        ArenaNode::Cached(cached) => {
            let old = old.ok_or_else(|| {
                format!(
                    "Memoized Lua subtree {:?} has no previous node",
                    cached.memo_key
                )
            })?;
            if old.memo_key.as_ref() != Some(&cached.memo_key)
                || !old.identity_matches(&cached.element_type, cached.key.as_deref())
            {
                return Err(format!(
                    "Memoized Lua subtree {:?} moved or changed identity",
                    cached.memo_key
                ));
            }
            handle_aliases.insert(handle_token, old.id);
            Ok(old)
        }
        ArenaNode::Pending(mut next) => {
            let old = match old {
                Some(old) if old.identity_matches(&next.element_type, next.key.as_deref()) => {
                    Some(old)
                }
                Some(old) => {
                    tree.destroy_element(old.id);
                    None
                }
                None => None,
            };
            let (id, old_children) = if let Some(mut old) = old {
                (old.id, std::mem::take(&mut old.children))
            } else {
                let id = *next_id;
                *next_id += 1;
                tree.create_element(id, next.element_type.clone());
                (id, Vec::new())
            };

            let next_children = std::mem::take(&mut next.children);
            let mut same_order = old_children.len() == next_children.len();
            if same_order {
                for (old, index) in old_children.iter().zip(&next_children) {
                    if !arena.identity_matches(*index, old)? {
                        same_order = false;
                        break;
                    }
                }
            }

            let mut children = Vec::with_capacity(next_children.len());
            if same_order {
                for (child_index, previous) in next_children.into_iter().zip(old_children) {
                    children.push(reconcile_node(
                        tree,
                        next_id,
                        Some(previous),
                        arena,
                        child_index,
                        handle_aliases,
                    )?);
                }
            } else {
                let mut keyed = HashMap::new();
                let mut unkeyed = VecDeque::new();
                for child in old_children {
                    if let Some(key) = child.key.clone() {
                        keyed.insert(key, child);
                    } else {
                        unkeyed.push_back(child);
                    }
                }

                for child_index in next_children {
                    let previous = match arena.key(child_index)? {
                        Some(key) => keyed.remove(key),
                        None => unkeyed.pop_front(),
                    };
                    children.push(reconcile_node(
                        tree,
                        next_id,
                        previous,
                        arena,
                        child_index,
                        handle_aliases,
                    )?);
                }
                for child in keyed.into_values().chain(unkeyed) {
                    tree.destroy_element(child.id);
                }
            }

            let event_types = next.events.keys().cloned().collect();
            tree.update_element(
                id,
                next.style,
                next.content,
                event_types,
                next.custom_props,
                next.auto_focus,
                next.test_id,
            );
            tree.replace_children(id, children.iter().map(|child| child.id).collect());
            Ok(LuaNode {
                id,
                handle_token,
                element_type: next.element_type,
                key: next.key,
                events: next.events,
                children,
                memo_key: next.memo_key,
            })
        }
    }
}

fn collect_host_handles(node: &LuaNode, handles: &mut HostHandleMap) {
    handles.insert(node.handle_token, node.id);
    for child in &node.children {
        collect_host_handles(child, handles);
    }
}

fn find_host_node(node: &mut LuaNode, id: u64) -> Option<&mut LuaNode> {
    if node.id == id {
        return Some(node);
    }
    node.children
        .iter_mut()
        .find_map(|child| find_host_node(child, id))
}

fn collect_handlers(node: &LuaNode, handlers: &mut HashMap<(u64, String), Function>) {
    for (event_type, handler) in &node.events {
        handlers.insert((node.id, event_type.clone()), handler.clone());
    }
    for child in &node.children {
        collect_handlers(child, handlers);
    }
}

fn event_table(lua: &Lua, payload: &EventPayload) -> mlua::Result<Table> {
    let event = lua.create_table()?;
    event.set("elementId", payload.element_id)?;
    event.set("type", payload.event_type.as_str())?;
    event.set("x", payload.x)?;
    event.set("y", payload.y)?;
    event.set("button", payload.button)?;
    event.set("clickCount", payload.click_count)?;
    event.set("isRightClick", payload.is_right_click)?;
    event.set("pressedButton", payload.pressed_button)?;
    event.set("key", payload.key.as_deref())?;
    event.set("keyChar", payload.key_char.as_deref())?;
    event.set("isHeld", payload.is_held)?;
    event.set("deltaX", payload.delta_x)?;
    event.set("deltaY", payload.delta_y)?;
    event.set("precise", payload.precise)?;
    event.set("touchPhase", payload.touch_phase.as_deref())?;
    event.set("hovered", payload.hovered)?;
    event.set("value", payload.value.as_deref())?;
    event.set("oldLine", payload.old_line)?;
    event.set("newLine", payload.new_line)?;
    event.set("startIndex", payload.start_index)?;
    event.set("endIndex", payload.end_index)?;
    event.set("matchCount", payload.match_count)?;
    if let Some(modifiers) = &payload.modifiers {
        let value = lua.create_table()?;
        value.set("shift", modifiers.shift)?;
        value.set("ctrl", modifiers.ctrl)?;
        value.set("alt", modifiers.alt)?;
        value.set("cmd", modifiers.cmd)?;
        event.set("modifiers", value)?;
    }
    Ok(event)
}

fn lua_error(error: mlua::Error) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempLuaApp(PathBuf);

    impl TempLuaApp {
        fn new() -> Self {
            static NEXT_TEMP_APP: std::sync::atomic::AtomicU64 =
                std::sync::atomic::AtomicU64::new(1);
            let unique = format!(
                "gpuix-lua-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_TEMP_APP.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            );
            let path = std::env::temp_dir().join(unique);
            std::fs::create_dir_all(path.join("components")).unwrap();
            Self(path)
        }

        fn write(&self, relative: &str, source: &str) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::write(&path, source).unwrap();
            path
        }
    }

    impl Drop for TempLuaApp {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn solid_show_routes_pending_subtrees_to_each_window() {
        let mut runtime = LuaRuntime::load_source_unrendered(
            r#"--!gpuix solid
                local solid = require("gpuix.solid")
                local Show = solid.Show
                local visible, set_visible = solid.create_signal(false)
                local app = gpuix.create_app()
                local function View()
                    return solid.mount(function()
                        return <div>
                            <div testId="toggle" onClick={function() set_visible(not visible()) end} />
                            <Show when={visible} render={function()
                                return <div>
                                    <Show when={visible} render={function()
                                        return <text>Shared {tostring(visible())}</text>
                                    end} />
                                </div>
                            end} />
                        </div>
                    end)
                end
                gpuix.define_window(app, { id = "main", render = View })
                gpuix.define_window(app, { id = "inspector", render = View })
                return app
            "#,
            None,
            true,
        ).unwrap();
        let mut main = RetainedTree::new();
        let mut inspector = RetainedTree::new();
        runtime.mount_window("main", &mut main).unwrap();
        runtime.mount_window("inspector", &mut inspector).unwrap();
        let element_id = main
            .elements
            .values()
            .find(|node| node.test_id.as_deref() == Some("toggle"))
            .unwrap()
            .id;
        let dirty = runtime
            .dispatch_window_event(
                "main",
                EventPayload {
                    element_id: element_id as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(dirty.len(), 2);
        runtime.mount_window("main", &mut main).unwrap();
        runtime.mount_window("inspector", &mut inspector).unwrap();
        assert!(has_text(&main, "Shared true"));
        assert!(has_text(&inspector, "Shared true"));
        assert!(runtime.host_updates.lock().unwrap().is_empty());
    }

    #[test]
    fn solid_controls_track_accessors_without_hooks_or_rebuilding() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_luax(r#"--!gpuix solid
            local solid = require("gpuix.solid")
            local Button = require("gpuix.solid.button")
            local Checkbox = require("gpuix.solid.checkbox")
            local RadioGroup = require("gpuix.solid.radio_group")
            local Select = require("gpuix.solid.select")
            local checked, set_checked = solid.create_signal(false)
            local disabled, set_disabled = solid.create_signal(false)
            local value, set_value = solid.create_signal("a")
            local options, set_options = solid.create_signal({
                {value="a",label="Alpha"}, {value="b",label="Beta"}, {value="c",label="Blocked",disabled=true}
            })
            widget_renders, button_clicks = 0, 0
            return function()
                widget_renders = widget_renders + 1
                return solid.mount(function()
                    return <div>
                        <Button testId="button" disabled={disabled}
                            onClick={function() button_clicks = button_clicks + 1 end}><text>Press</text></Button>
                        <Checkbox testId="checkbox" checked={checked} disabled={disabled}
                            onCheckedChange={set_checked} label="Enabled" />
                        <Checkbox testId="local-checkbox" defaultChecked />
                        <RadioGroup testId="radio" options={options} value={value} onValueChange={set_value} />
                        <Select testId="select" options={options} value={value} onValueChange={set_value} />
                        <div testId="disable" onClick={function() set_disabled(not disabled()) end} />
                        <div testId="reorder" onClick={function()
                            set_options({{value="b",label="Bee"}, {value="a",label="Alpha"}})
                        end} />
                    </div>
                end)
            end
        "#, &mut tree).unwrap();
        fn id(tree: &RetainedTree, name: &str) -> u64 {
            tree.elements
                .values()
                .find(|node| node.test_id.as_deref() == Some(name))
                .unwrap()
                .id
        }
        let button = id(&tree, "button");
        let radio_b = id(&tree, "radio-b");
        let checkbox = id(&tree, "checkbox");
        dispatch_by_test_id(&mut runtime, &mut tree, "checkbox-indicator").unwrap();
        dispatch_by_test_id(&mut runtime, &mut tree, "button").unwrap();
        assert_eq!(
            runtime.lua.globals().get::<i64>("button_clicks").unwrap(),
            1
        );
        dispatch_by_test_id(&mut runtime, &mut tree, "disable").unwrap();
        assert_eq!(
            tree.elements[&button].custom_props["tabIndex"],
            serde_json::json!(-1)
        );
        assert_eq!(
            tree.elements[&checkbox].custom_props["tabIndex"],
            serde_json::json!(-1)
        );
        dispatch_by_test_id(&mut runtime, &mut tree, "button").unwrap();
        assert_eq!(
            runtime.lua.globals().get::<i64>("button_clicks").unwrap(),
            1
        );
        dispatch_by_test_id(&mut runtime, &mut tree, "radio-b").unwrap();
        assert_eq!(
            tree.elements[&radio_b].custom_props["tabIndex"],
            serde_json::json!(0)
        );
        runtime
            .dispatch_event(
                EventPayload {
                    element_id: radio_b as f64,
                    event_type: "keyDown".into(),
                    key: Some("right".into()),
                    ..Default::default()
                },
                &mut tree,
            )
            .unwrap();
        let radio_a = id(&tree, "radio-a");
        assert_eq!(
            tree.elements[&radio_a].custom_props["tabIndex"],
            serde_json::json!(0)
        );
        dispatch_by_test_id(&mut runtime, &mut tree, "select").unwrap();
        assert!(has_test_id(&tree, "select-content"));
        dispatch_by_test_id(&mut runtime, &mut tree, "select-option-c").unwrap();
        assert!(has_test_id(&tree, "select-content"));
        dispatch_by_test_id(&mut runtime, &mut tree, "select-option-a").unwrap();
        assert!(!has_test_id(&tree, "select-content"));
        dispatch_by_test_id(&mut runtime, &mut tree, "reorder").unwrap();
        assert!(tree.elements.contains_key(&radio_b));
        assert!(has_text(&tree, "Bee"));
        assert_eq!(
            runtime.lua.globals().get::<i64>("widget_renders").unwrap(),
            1
        );
        assert_eq!(id(&tree, "button"), button);
        runtime.unmount_window(DEFAULT_ROOT_ID);
    }

    #[test]
    fn solid_host_props_update_without_rebuilding_and_nil_removes_them() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_luax(r#"--!gpuix solid
            local solid = require("gpuix.solid")
            local value, set_value = solid.create_signal("hello")
            local locked, set_locked = solid.create_signal(false)
            prop_renders = 0
            return function()
                prop_renders = prop_renders + 1
                return solid.mount(function()
                    return <div>
                        <input testId="field" value={value()} readOnly={locked()} placeholder="Write" />
                        <div testId="update" onClick={function()
                            solid.batch(function() set_value(""); set_locked(true) end)
                        end} />
                        <div testId="remove" onClick={function() set_value(nil) end} />
                    </div>
                end)
            end
        "#, &mut tree).unwrap();
        let id = tree
            .elements
            .values()
            .find(|node| node.test_id.as_deref() == Some("field"))
            .unwrap()
            .id;
        assert_eq!(
            tree.elements[&id].custom_props["value"],
            serde_json::json!("hello")
        );
        dispatch_by_test_id(&mut runtime, &mut tree, "update").unwrap();
        assert_eq!(
            tree.elements[&id].custom_props["value"],
            serde_json::json!("")
        );
        assert_eq!(
            tree.elements[&id].custom_props["readOnly"],
            serde_json::json!(true)
        );
        assert_eq!(
            tree.elements[&id].custom_props["placeholder"],
            serde_json::json!("Write")
        );
        dispatch_by_test_id(&mut runtime, &mut tree, "remove").unwrap();
        assert!(!tree.elements[&id].custom_props.contains_key("value"));
        assert_eq!(runtime.lua.globals().get::<i64>("prop_renders").unwrap(), 1);
        runtime
            .lua
            .load("assert(not pcall(gpuix.set_prop, 1, 'onClick', true))")
            .exec()
            .unwrap();
        runtime.unmount_window(DEFAULT_ROOT_ID);
    }

    #[test]
    fn solid_workspace_exercises_shared_store_lists_and_panel_lifetimes() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/solid-workspace/main.luax");
        let mut runtime = LuaRuntime::load_application_file(&path).unwrap();
        let mut main = RetainedTree::new();
        let mut inspector = RetainedTree::new();
        runtime.mount_window("main", &mut main).unwrap();
        runtime.mount_window("inspector", &mut inspector).unwrap();
        fn event(
            runtime: &mut LuaRuntime,
            main: &mut RetainedTree,
            inspector: &mut RetainedTree,
            window: &str,
            test_id: &str,
            kind: &str,
            value: Option<String>,
        ) {
            let tree = if window == "main" {
                &*main
            } else {
                &*inspector
            };
            let id = tree
                .elements
                .values()
                .find(|node| node.test_id.as_deref() == Some(test_id))
                .unwrap()
                .id;
            let dirty = runtime
                .dispatch_window_event(
                    window,
                    EventPayload {
                        element_id: id as f64,
                        event_type: kind.into(),
                        value,
                        ..Default::default()
                    },
                )
                .unwrap();
            for root in dirty {
                let tree = if root == "main" {
                    &mut *main
                } else {
                    &mut *inspector
                };
                runtime.mount_window(&root, tree).unwrap();
            }
        }
        let original_root = main.root_id;
        let row = main
            .elements
            .values()
            .find(|node| node.test_id.as_deref() == Some("conversation-lists"))
            .unwrap()
            .id;
        event(
            &mut runtime,
            &mut main,
            &mut inspector,
            "main",
            "conversation-lists",
            "click",
            None,
        );
        assert!(has_text(&main, "Visits: 1"));
        assert!(has_text(&inspector, "Selected: lists"));
        event(
            &mut runtime,
            &mut main,
            &mut inspector,
            "inspector",
            "inspector-reverse",
            "click",
            None,
        );
        assert!(main.elements.contains_key(&row));
        assert!(has_text(&main, "Visits: 1"));
        event(
            &mut runtime,
            &mut main,
            &mut inspector,
            "main",
            "activity-click",
            "click",
            None,
        );
        assert!(has_text(&main, "Panel clicks: 1"));
        event(
            &mut runtime,
            &mut main,
            &mut inspector,
            "main",
            "toggle-activity",
            "click",
            None,
        );
        assert!(!has_test_id(&main, "activity-panel"));
        event(
            &mut runtime,
            &mut main,
            &mut inspector,
            "main",
            "toggle-activity",
            "click",
            None,
        );
        assert!(has_text(&main, "Panel clicks: 0"));
        event(
            &mut runtime,
            &mut main,
            &mut inspector,
            "main",
            "composer",
            "change",
            Some("Hello Solid".into()),
        );
        assert!(has_text(&inspector, "Draft: Hello Solid"));
        let composer_id = main
            .elements
            .values()
            .find(|node| node.test_id.as_deref() == Some("composer"))
            .unwrap()
            .id;
        assert_eq!(
            main.elements[&composer_id].custom_props["value"],
            serde_json::json!("Hello Solid")
        );
        event(
            &mut runtime,
            &mut main,
            &mut inspector,
            "main",
            "send",
            "click",
            None,
        );
        assert!(has_test_id(&main, "conversation-message-1"));
        assert!(has_text(&main, "Messages: 4"));
        assert!(has_text(&main, "2. Sent message-1"));
        assert!(has_text(&inspector, "Messages: 4"));
        assert!(has_text(&inspector, "Selected: message-1"));
        assert_eq!(
            main.elements[&composer_id].custom_props["value"],
            serde_json::json!("")
        );
        assert_eq!(main.root_id, original_root);
        runtime.unmount_window("main");
        runtime.unmount_window("inspector");
        assert!(runtime.stores.lock().unwrap().entries["solid-workspace"]
            .listeners
            .is_empty());
    }

    #[test]
    fn solid_memos_cache_batch_track_dependencies_and_dispose() {
        let mut tree = RetainedTree::new();
        let runtime =
            LuaRuntime::load("return function() return gpuix.div {} end", &mut tree).unwrap();
        runtime
            .lua
            .load(
                r#"
            local solid = require("gpuix.solid")
            assert(not pcall(solid.create_memo, function() return 1 end))
            local value, set_value = solid.create_signal(1)
            local other, set_other = solid.create_signal(10)
            local first, set_first = solid.create_signal(true)
            local runs, effects, cleanups = 0, 0, 0
            local memo, owner = solid.create_root(function()
                local selected = solid.create_memo(function()
                    runs = runs + 1
                    solid.on_cleanup(function() cleanups = cleanups + 1 end)
                    return first() and value() or other()
                end)
                solid.create_effect(function() selected(); effects = effects + 1 end)
                solid.create_effect(function() selected() end)
                return selected
            end)
            assert(memo() == 1 and memo() == 1 and runs == 1)
            set_other(11)
            assert(runs == 1)
            solid.batch(function() set_value(2); set_value(3) end)
            assert(memo() == 3 and runs == 2 and effects == 2)
            solid.batch(function()
                set_value(4)
                assert(memo() == 4)
                assert(memo() == 4 and runs == 3)
            end)
            set_first(false)
            assert(memo() == 11 and runs == 4)
            set_value(5)
            assert(runs == 4)
            solid.dispose(owner)
            assert(cleanups == 4)
            set_other(12)
            assert(memo() == 11 and runs == 4)

            local parity_effects = 0
            local _, parity_owner = solid.create_root(function()
                local parity = solid.create_memo(function() return value() % 2 end)
                solid.create_effect(function() parity(); parity_effects = parity_effects + 1 end)
            end)
            set_value(7)
            assert(parity_effects == 1)
            set_value(8)
            assert(parity_effects == 2)
            solid.dispose(parity_owner)

            local derived, diamond_owner = solid.create_root(function()
                local left = solid.create_memo(function() return value() * 2 end)
                local right = solid.create_memo(function() return value() * 3 end)
                local total = solid.create_memo(function() return left() + right() end)
                solid.create_effect(function()
                    assert(total() == value() * 5)
                end)
                return total
            end)
            for next_value = 9, 30 do
                solid.batch(function()
                    set_value(next_value)
                    assert(derived() == next_value * 5)
                end)
            end
            solid.dispose(diamond_owner)
            local result, result_owner = solid.create_root(function()
                return solid.create_memo(function()
                    if value() == 30 then return nil end
                    return function() return value() end
                end)
            end)
            assert(result() == nil)
            set_value(31)
            assert(type(result()) == "function" and result()() == 31)
            solid.dispose(result_owner)
        "#,
            )
            .exec()
            .unwrap();
    }

    #[test]
    fn solid_store_selectors_skip_equal_values_and_cleanup_branches() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_luax(
            r#"--!gpuix solid
                local solid = require("gpuix.solid")
                local Show = solid.Show
                local store = gpuix.create_store("solid-store", function(state, action)
                    return {count = state.count + action}
                end, {count=0})
                local visible, set_visible = solid.create_signal(true)
                store_effects, store_renders = 0, 0
                return function()
                    store_renders = store_renders + 1
                    return solid.mount(function()
                        return <div>
                            <div testId="two" onClick={function() store.dispatch(2) end} />
                            <div testId="one" onClick={function() store.dispatch(1) end} />
                            <div testId="toggle" onClick={function() set_visible(not visible()) end} />
                            <Show when={visible} render={function()
                                local parity = solid.use_store(store,
                                    function(state) return {value=state.count % 2} end,
                                    function(previous, next) return previous.value == next.value end)
                                solid.create_effect(function()
                                    parity()
                                    store_effects = store_effects + 1
                                end)
                                return <text>Parity {parity().value}</text>
                            end} />
                        </div>
                    end)
                end
            "#,
            &mut tree,
        ).unwrap();
        assert_eq!(
            runtime.stores.lock().unwrap().entries["solid-store"]
                .listeners
                .len(),
            1
        );
        assert!(!dispatch_by_test_id(&mut runtime, &mut tree, "two").unwrap());
        assert_eq!(
            runtime.lua.globals().get::<i64>("store_effects").unwrap(),
            1
        );
        dispatch_by_test_id(&mut runtime, &mut tree, "one").unwrap();
        assert!(has_text(&tree, "Parity 1"));
        assert_eq!(
            runtime.lua.globals().get::<i64>("store_effects").unwrap(),
            2
        );
        dispatch_by_test_id(&mut runtime, &mut tree, "toggle").unwrap();
        assert!(runtime.stores.lock().unwrap().entries["solid-store"]
            .listeners
            .is_empty());
        dispatch_by_test_id(&mut runtime, &mut tree, "one").unwrap();
        assert_eq!(
            runtime.lua.globals().get::<i64>("store_effects").unwrap(),
            2
        );
        dispatch_by_test_id(&mut runtime, &mut tree, "toggle").unwrap();
        assert!(has_text(&tree, "Parity 0"));
        assert_eq!(
            runtime.lua.globals().get::<i64>("store_renders").unwrap(),
            1
        );
        runtime.unmount_window(DEFAULT_ROOT_ID);
        assert!(runtime.stores.lock().unwrap().entries["solid-store"]
            .listeners
            .is_empty());
    }

    #[test]
    fn solid_store_shares_state_with_react_style_window_hooks() {
        let mut runtime = LuaRuntime::load_source_unrendered(
            r#"--!gpuix solid
                local solid = require("gpuix.solid")
                local store = gpuix.create_store("shared-solid", function(state, action)
                    return {count=state.count+1}
                end, {count=0})
                local app = gpuix.create_app()
                solid_renders, hook_renders = 0, 0
                gpuix.define_window(app, {id="main", render=function()
                    solid_renders = solid_renders + 1
                    return solid.mount(function()
                        local state = solid.use_store(store)
                        return <div>
                            <text>Solid {state().count}</text>
                            <div testId="increment" onClick={function() store.dispatch({}) end} />
                        </div>
                    end)
                end})
                gpuix.define_window(app, {id="inspector", render=function()
                    hook_renders = hook_renders + 1
                    local count = store.use_state(function(state) return state.count end)
                    return gpuix.text("Hook " .. count)
                end})
                return app
            "#,
            None,
            true,
        )
        .unwrap();
        let mut main = RetainedTree::new();
        let mut inspector = RetainedTree::new();
        runtime.mount_window("main", &mut main).unwrap();
        runtime.mount_window("inspector", &mut inspector).unwrap();
        let element_id = main
            .elements
            .values()
            .find(|node| node.test_id.as_deref() == Some("increment"))
            .unwrap()
            .id;
        let dirty = runtime
            .dispatch_window_event(
                "main",
                EventPayload {
                    element_id: element_id as f64,
                    event_type: "click".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(dirty.len(), 2);
        runtime.mount_window("main", &mut main).unwrap();
        runtime.mount_window("inspector", &mut inspector).unwrap();
        assert!(has_text(&main, "Solid 1"));
        assert!(has_text(&inspector, "Hook 1"));
        assert_eq!(
            runtime.lua.globals().get::<i64>("solid_renders").unwrap(),
            1
        );
        assert_eq!(runtime.lua.globals().get::<i64>("hook_renders").unwrap(), 2);
        runtime.unmount_window("main");
        assert!(runtime.stores.lock().unwrap().entries["shared-solid"]
            .listeners
            .is_empty());
    }

    #[test]
    fn solid_counter_example_supports_keyed_rows() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_luax(
            include_str!("../../../examples/solid-counter.luax"),
            &mut tree,
        )
        .unwrap();
        assert!(has_text(&tree, "Row 1, position 1: 0 clicks"));
        assert!(has_text(&tree, "Row 3, position 3: 0 clicks"));
        dispatch_by_test_id(&mut runtime, &mut tree, "solid-counter").unwrap();
        assert!(has_text(&tree, "Count: 1"));
    }

    #[test]
    fn solid_index_preserves_positions_and_disposes_tail_rows() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_luax(
            r#"--!gpuix solid
                local solid = require("gpuix.solid")
                local Index = solid.Index
                local values, set_values = solid.create_signal({"A", "B"})
                index_mounts, index_cleanups, index_renders = 0, 0, 0
                return function()
                    index_renders = index_renders + 1
                    return solid.mount(function()
                        return <div>
                            <div testId="swap" onClick={function() set_values({"B", "A"}) end} />
                            <div testId="shrink" onClick={function() set_values({"C"}) end} />
                            <div testId="grow" onClick={function()
                                solid.batch(function()
                                    set_values({})
                                    set_values({"C", "C", "D"})
                                end)
                            end} />
                            <div testId="invalid" onClick={function() set_values({[2]="bad"}) end} />
                            <div testId="empty" onClick={function() set_values({}) end} />
                            <Index testId="slots" each={values} render={function(item, index)
                                assert(type(index) == "number")
                                index_mounts = index_mounts + 1
                                solid.on_cleanup(function() index_cleanups = index_cleanups + 1 end)
                                local clicks, set_clicks = solid.create_signal(0)
                                return <div testId={"slot-" .. index}
                                    onClick={function() set_clicks(clicks() + 1) end}>
                                    <text>{index}:{item()}:{clicks()}</text>
                                </div>
                            end} />
                        </div>
                    end)
                end
            "#,
            &mut tree,
        ).unwrap();
        let ids = tree
            .elements
            .values()
            .find(|node| node.test_id.as_deref() == Some("slots"))
            .unwrap()
            .children
            .clone();
        dispatch_by_test_id(&mut runtime, &mut tree, "slot-1").unwrap();
        dispatch_by_test_id(&mut runtime, &mut tree, "swap").unwrap();
        assert!(has_text(&tree, "1:B:1"));
        assert!(has_text(&tree, "2:A:0"));
        let slot = tree
            .elements
            .values()
            .find(|node| node.test_id.as_deref() == Some("slots"))
            .unwrap();
        assert_eq!(slot.children, ids);
        assert_eq!(runtime.lua.globals().get::<i64>("index_mounts").unwrap(), 2);
        dispatch_by_test_id(&mut runtime, &mut tree, "shrink").unwrap();
        assert!(has_text(&tree, "1:C:1"));
        assert!(!tree.elements.contains_key(&ids[1]));
        assert!(!runtime.roots[DEFAULT_ROOT_ID]
            .handlers
            .contains_key(&(ids[1], "click".to_string())));
        assert_eq!(
            runtime.lua.globals().get::<i64>("index_cleanups").unwrap(),
            1
        );
        dispatch_by_test_id(&mut runtime, &mut tree, "grow").unwrap();
        assert!(has_text(&tree, "1:C:1"));
        assert!(has_text(&tree, "2:C:0"));
        assert!(has_text(&tree, "3:D:0"));
        assert_eq!(runtime.lua.globals().get::<i64>("index_mounts").unwrap(), 4);
        assert!(dispatch_by_test_id(&mut runtime, &mut tree, "invalid").is_err());
        assert!(has_text(&tree, "1:C:1"));
        dispatch_by_test_id(&mut runtime, &mut tree, "empty").unwrap();
        assert!(!has_test_id(&tree, "slot-1"));
        assert_eq!(
            runtime.lua.globals().get::<i64>("index_cleanups").unwrap(),
            4
        );
        dispatch_by_test_id(&mut runtime, &mut tree, "grow").unwrap();
        assert!(has_text(&tree, "1:C:0"));
        assert_eq!(
            runtime.lua.globals().get::<i64>("index_renders").unwrap(),
            1
        );
        runtime.unmount_window(DEFAULT_ROOT_ID);
        assert_eq!(
            runtime.lua.globals().get::<i64>("index_cleanups").unwrap(),
            7
        );
    }

    #[test]
    fn solid_for_preserves_rows_and_updates_items_and_indices() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_luax(
            r#"--!gpuix solid
                local solid = require("gpuix.solid")
                local For = solid.For
                local items, set_items = solid.create_signal({{id="a", title="A"}, {id="b", title="B"}})
                mounts, cleanups = 0, 0
                return function()
                    return solid.mount(function()
                        return <div>
                            <div testId="reorder" onClick={function()
                                set_items({{id="b",title="Bee"}, {id="a",title="A"}})
                            end} />
                            <div testId="remove" onClick={function() set_items({{id="b",title="Bee"}}) end} />
                            <div testId="add" onClick={function()
                                solid.batch(function()
                                    set_items({})
                                    set_items({{id="a",title="Again"}, {id="b",title="Bee"}})
                                end)
                            end} />
                            <div testId="empty" onClick={function() set_items({}) end} />
                            <div testId="invalid" onClick={function()
                                set_items({{id="b",title="B"}, {id="b",title="Duplicate"}})
                            end} />
                            <For testId="rows" each={items} key={function(item) return item.id end}
                                render={function(item, index)
                                    mounts = mounts + 1
                                    solid.on_cleanup(function() cleanups = cleanups + 1 end)
                                    local clicks, set_clicks = solid.create_signal(0)
                                    return <div testId={item().id} onClick={function() set_clicks(clicks()+1) end}>
                                        <text>{item().title}:{index()}:{clicks()}</text>
                                    </div>
                                end} />
                        </div>
                    end)
                end
            "#,
            &mut tree,
        ).unwrap();
        let row_id = tree
            .elements
            .values()
            .find(|node| node.test_id.as_deref() == Some("a"))
            .unwrap()
            .id;
        dispatch_by_test_id(&mut runtime, &mut tree, "a").unwrap();
        dispatch_by_test_id(&mut runtime, &mut tree, "reorder").unwrap();
        assert!(has_text(&tree, "A:2:1"));
        assert!(has_text(&tree, "Bee:1:0"));
        assert!(tree.elements.contains_key(&row_id));
        assert_eq!(runtime.lua.globals().get::<i64>("mounts").unwrap(), 2);
        let slot = tree
            .elements
            .values()
            .find(|node| node.test_id.as_deref() == Some("rows"))
            .unwrap();
        assert_eq!(slot.children[1], row_id);
        dispatch_by_test_id(&mut runtime, &mut tree, "remove").unwrap();
        assert!(!tree.elements.contains_key(&row_id));
        assert_eq!(runtime.lua.globals().get::<i64>("cleanups").unwrap(), 1);
        dispatch_by_test_id(&mut runtime, &mut tree, "add").unwrap();
        assert!(has_text(&tree, "Again:1:0"));
        assert_eq!(runtime.lua.globals().get::<i64>("mounts").unwrap(), 3);
        assert!(dispatch_by_test_id(&mut runtime, &mut tree, "invalid").is_err());
        assert!(has_text(&tree, "Again:1:0"));
        dispatch_by_test_id(&mut runtime, &mut tree, "empty").unwrap();
        assert!(!has_test_id(&tree, "a"));
        assert!(!has_test_id(&tree, "b"));
        assert_eq!(runtime.lua.globals().get::<i64>("cleanups").unwrap(), 3);
    }

    #[test]
    fn solid_show_preserves_truthy_branches_and_disposes_removed_nodes() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_luax(
            r#"--!gpuix solid
                local solid = require("gpuix.solid")
                local Show = solid.Show
                local level, set_level = solid.create_signal(0)
                branch_mounts, branch_cleanups = 0, 0
                local renders = 0
                return function()
                    renders = renders + 1
                    return solid.mount(function()
                        return <div>
                            <div testId="toggle" onClick={function() set_level((level() + 1) % 3) end} />
                            <Show testId="slot" when={function() return level() > 0 end}
                                fallback={function() return <text>Hidden</text> end}
                                render={function()
                                    branch_mounts = branch_mounts + 1
                                    solid.on_cleanup(function() branch_cleanups = branch_cleanups + 1 end)
                                    local count, set_count = solid.create_signal(0)
                                    return <div testId="branch" onClick={function() set_count(count() + 1) end}>
                                        <text>Local {count()}</text>
                                        <Show when={function() return true end} render={function()
                                            return <text>Nested {count()}</text>
                                        end} />
                                    </div>
                                end} />
                            <text>Renders {renders}</text>
                        </div>
                    end)
                end
            "#,
            &mut tree,
        ).unwrap();
        let root = tree.root_id;
        assert!(has_text(&tree, "Hidden"));
        dispatch_by_test_id(&mut runtime, &mut tree, "toggle").unwrap();
        assert!(has_text(&tree, "Local 0"));
        let branch = tree
            .elements
            .values()
            .find(|node| node.test_id.as_deref() == Some("branch"))
            .unwrap()
            .id;
        dispatch_by_test_id(&mut runtime, &mut tree, "branch").unwrap();
        assert!(has_text(&tree, "Nested 1"));
        dispatch_by_test_id(&mut runtime, &mut tree, "toggle").unwrap();
        assert!(tree.elements.contains_key(&branch));
        assert!(has_text(&tree, "Local 1"));
        dispatch_by_test_id(&mut runtime, &mut tree, "toggle").unwrap();
        assert!(!tree.elements.contains_key(&branch));
        assert!(has_text(&tree, "Hidden"));
        assert!(!runtime.roots[DEFAULT_ROOT_ID]
            .handlers
            .contains_key(&(branch, "click".to_string())));
        runtime
            .lua
            .load("assert(branch_mounts == 1 and branch_cleanups == 1)")
            .exec()
            .unwrap();
        dispatch_by_test_id(&mut runtime, &mut tree, "toggle").unwrap();
        assert!(has_text(&tree, "Local 0"));
        assert!(has_text(&tree, "Renders 1"));
        assert_eq!(tree.root_id, root);
        runtime.unmount_window(DEFAULT_ROOT_ID);
        runtime
            .lua
            .load("assert(branch_mounts == 2 and branch_cleanups == 2)")
            .exec()
            .unwrap();
    }

    #[test]
    fn solid_luax_tracks_text_and_style_without_component_renders() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_luax(
            r#"--!gpuix solid
                local solid = require("gpuix.solid")
                local count, set_count = solid.create_signal(0)
                local runs = 0
                return function()
                    runs = runs + 1
                    return solid.mount(function()
                        return <div testId="increment" style={{ width = 100 + count() }}
                            onClick={function() set_count(count() + 1) end}>
                            <text>Count {count()}</text>
                            <text testId="styled-text" style={{ height = 20 + count() }}>Value {count()}</text>
                            <text content={"Prop " .. count()} />
                            <text>Runs {runs}</text>
                        </div>
                    end)
                end
            "#,
            &mut tree,
        ).unwrap();
        let root = tree.root_id.unwrap();
        let children = tree.elements[&root].children.clone();
        assert!(dispatch_by_test_id(&mut runtime, &mut tree, "increment").unwrap());
        assert!(has_text(&tree, "Count 1"));
        assert!(has_text(&tree, "Value 1"));
        assert!(has_text(&tree, "Prop 1"));
        assert!(has_text(&tree, "Runs 1"));
        assert_eq!(tree.elements[&root].children, children);
        assert!(matches!(
            tree.elements[&root].style.as_ref().unwrap().width,
            Some(crate::style::DimensionValue::Pixels(101.0))
        ));
    }

    #[test]
    fn solid_bindings_update_native_nodes_without_rerendering() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local solid = require("gpuix.solid")
                local count, set_count = solid.create_signal(0)
                test_set_count = set_count
                test_cleanups = 0
                local renders = 0
                return function()
                    renders = renders + 1
                    return solid.mount(function()
                        solid.on_cleanup(function() test_cleanups = test_cleanups + 1 end)
                        local label = solid.text(function() return "Count " .. count() end)
                        local root = gpuix.div {
                            testId = "counter",
                            onClick = function() set_count(function(value) return value + 1 end) end,
                            children = { label, gpuix.text("Renders " .. renders) },
                        }
                        solid.bind_style(root, function()
                            return { width = 100 + count(), height = 40 }
                        end)
                        return root
                    end)
                end
            "#,
            &mut tree,
        ).unwrap();
        let root = tree.root_id.unwrap();
        let children = tree.elements[&root].children.clone();
        assert!(has_text(&tree, "Count 0"));
        assert!(dispatch_by_test_id(&mut runtime, &mut tree, "counter").unwrap());
        assert!(dispatch_by_test_id(&mut runtime, &mut tree, "counter").unwrap());
        assert!(has_text(&tree, "Count 2"));
        assert!(has_text(&tree, "Renders 1"));
        assert_eq!(tree.root_id, Some(root));
        assert_eq!(tree.elements[&root].children, children);
        assert!(matches!(
            tree.elements[&root].style.as_ref().unwrap().width,
            Some(crate::style::DimensionValue::Pixels(102.0))
        ));
        runtime.unmount_window(DEFAULT_ROOT_ID);
        runtime
            .lua
            .load("test_set_count(3); assert(test_cleanups == 1)")
            .exec()
            .unwrap();
        assert!(runtime.host_updates.lock().unwrap().is_empty());
    }

    #[test]
    fn solid_signals_track_batch_and_dispose_owned_effects() {
        let lua = Lua::new();
        install_builtin_modules(&lua).unwrap();
        lua.load(
            r#"
            local solid = require("gpuix.solid")
            local value, set_value = solid.create_signal(0)
            local enabled, set_enabled = solid.create_signal(true)
            local runs, seen, cleaned = 0, 0, 0
            local _, owner = solid.create_root(function()
                solid.create_effect(function()
                    runs = runs + 1
                    seen = enabled() and value() or -1
                    solid.on_cleanup(function() cleaned = cleaned + 1 end)
                end)
            end)
            solid.batch(function() set_value(1) set_value(2) end)
            assert(runs == 2 and seen == 2 and cleaned == 1)
            set_enabled(false)
            set_value(3)
            assert(runs == 3 and seen == -1)
            solid.dispose(owner)
            solid.dispose(owner)
            set_enabled(true)
            assert(runs == 3 and cleaned == 3)
            assert(not pcall(function() solid.create_effect(function() end) end))
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn state_update_reconciles_directly_into_retained_tree() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local ui = gpuix
                return function()
                    local count, set_count = ui.use_state(0)
                    return ui.div {
                        testId = "root",
                        ui.text("Count: " .. count),
                        ui.div {
                            key = "button",
                            testId = "button",
                            onClick = function() set_count(function(value) return value + 1 end) end,
                            ui.text("Increment"),
                        },
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        let root_id = tree.root_id.unwrap();
        let button_id = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("button"))
            .unwrap()
            .id;
        let before_ids: HashSet<u64> = tree.elements.keys().copied().collect();
        let changed = runtime
            .dispatch_event(
                EventPayload {
                    element_id: button_id as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
                &mut tree,
            )
            .unwrap();

        assert!(changed);
        assert_eq!(tree.root_id, Some(root_id));
        assert_eq!(before_ids, tree.elements.keys().copied().collect());
        assert!(tree
            .elements
            .values()
            .any(|element| element.content.as_deref() == Some("Count: 1")));
    }

    #[test]
    fn reducer_ref_memo_and_callback_follow_react_style_dependencies() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local ui = gpuix
                memo_runs = 0
                callback_changed = 0
                setter_changed = 0
                dispatch_changed = 0
                local previous_callback = nil
                local previous_setter = nil
                local previous_dispatch = nil

                return function()
                    local count, dispatch = ui.use_reducer(function(state, action)
                        return state + action
                    end, 0)
                    local unrelated, set_unrelated = ui.use_state(0)
                    if previous_setter ~= nil and previous_setter ~= set_unrelated then
                        setter_changed = setter_changed + 1
                    end
                    if previous_dispatch ~= nil and previous_dispatch ~= dispatch then
                        dispatch_changed = dispatch_changed + 1
                    end
                    previous_setter = set_unrelated
                    previous_dispatch = dispatch
                    local renders = ui.use_ref(0)
                    renders.current = renders.current + 1
                    local doubled = ui.use_memo(function()
                        memo_runs = memo_runs + 1
                        return count * 2
                    end, { count })
                    local read_count = ui.use_callback(function() return count end, { count })
                    if previous_callback ~= nil and previous_callback ~= read_count then
                        callback_changed = callback_changed + 1
                    end
                    previous_callback = read_count

                    return ui.div {
                        ui.text("values:" .. count .. ":" .. doubled .. ":" .. renders.current .. ":" .. read_count()),
                        ui.div { testId = "unrelated", onClick = function() set_unrelated(unrelated + 1) end },
                        ui.div { testId = "reduce", onClick = function() dispatch(2) end },
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        assert!(has_text(&tree, "values:0:0:1:0"));
        assert_eq!(runtime.lua.globals().get::<i64>("memo_runs").unwrap(), 1);
        dispatch_by_test_id(&mut runtime, &mut tree, "unrelated").unwrap();
        assert!(has_text(&tree, "values:0:0:2:0"));
        assert_eq!(runtime.lua.globals().get::<i64>("memo_runs").unwrap(), 1);
        assert_eq!(
            runtime
                .lua
                .globals()
                .get::<i64>("callback_changed")
                .unwrap(),
            0
        );
        assert_eq!(
            runtime.lua.globals().get::<i64>("setter_changed").unwrap(),
            0
        );
        assert_eq!(
            runtime
                .lua
                .globals()
                .get::<i64>("dispatch_changed")
                .unwrap(),
            0
        );

        dispatch_by_test_id(&mut runtime, &mut tree, "reduce").unwrap();
        assert!(has_text(&tree, "values:2:4:3:2"));
        assert_eq!(runtime.lua.globals().get::<i64>("memo_runs").unwrap(), 2);
        assert_eq!(
            runtime
                .lua
                .globals()
                .get::<i64>("callback_changed")
                .unwrap(),
            1
        );
        assert_eq!(
            runtime.lua.globals().get::<i64>("setter_changed").unwrap(),
            0
        );
        assert_eq!(
            runtime
                .lua
                .globals()
                .get::<i64>("dispatch_changed")
                .unwrap(),
            0
        );
    }

    #[test]
    fn effects_run_after_commit_rerender_and_cleanup_on_change_and_unmount() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local ui = gpuix
                effect_runs = 0
                cleanup_runs = 0

                local function Child(props)
                    ui.use_effect(function()
                        effect_runs = effect_runs + 1
                        props.set_seen(props.value)
                        return function() cleanup_runs = cleanup_runs + 1 end
                    end, { props.value })
                    return ui.text("child:" .. props.value)
                end

                return function()
                    local value, set_value = ui.use_state(0)
                    local seen, set_seen = ui.use_state(-1)
                    local visible, set_visible = ui.use_state(true)
                    return ui.div {
                        ui.text("seen:" .. seen),
                        visible and ui.h(Child, { value = value, set_seen = set_seen }) or nil,
                        ui.div { testId = "change", onClick = function() set_value(value + 1) end },
                        ui.div { testId = "hide", onClick = function() set_visible(false) end },
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        assert!(has_text(&tree, "seen:0"));
        assert_eq!(runtime.lua.globals().get::<i64>("effect_runs").unwrap(), 1);
        assert_eq!(runtime.lua.globals().get::<i64>("cleanup_runs").unwrap(), 0);

        dispatch_by_test_id(&mut runtime, &mut tree, "change").unwrap();
        assert!(has_text(&tree, "seen:1"));
        assert_eq!(runtime.lua.globals().get::<i64>("effect_runs").unwrap(), 2);
        assert_eq!(runtime.lua.globals().get::<i64>("cleanup_runs").unwrap(), 1);

        dispatch_by_test_id(&mut runtime, &mut tree, "hide").unwrap();
        assert!(!has_text(&tree, "child:1"));
        assert_eq!(runtime.lua.globals().get::<i64>("effect_runs").unwrap(), 2);
        assert_eq!(runtime.lua.globals().get::<i64>("cleanup_runs").unwrap(), 2);
    }

    #[test]
    fn redux_store_selectors_skip_unchanged_updates_and_invalidate_memoized_components() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_luax(
            r#"
                local ui = gpuix
                renders = 0
                store = ui.create_store("counter", function(state, action)
                    if action.type == "increment" then
                        return { count = state.count + 1, noise = state.noise }
                    end
                    if action.type == "noise" then
                        return { count = state.count, noise = state.noise + 1 }
                    end
                    return state
                end, { count = 0, noise = 0 })

                local function Counter()
                    renders = renders + 1
                    local count = store.use_state(function(state) return state.count end)
                    return <div>
                        <text>count:{count}</text>
                        <div testId="increment" onClick={function()
                            store.dispatch({ type = "increment" })
                        end} />
                        <div testId="noise" onClick={function()
                            store.dispatch({ type = "noise" })
                        end} />
                    </div>
                end

                return function()
                    return ui.memo("counter", true, function()
                        return ui.h(Counter, {})
                    end)
                end
            "#,
            &mut tree,
        )
        .unwrap();

        assert!(has_text(&tree, "count:0"));
        assert_eq!(runtime.lua.globals().get::<i64>("renders").unwrap(), 1);
        assert!(!dispatch_by_test_id(&mut runtime, &mut tree, "noise").unwrap());
        assert_eq!(runtime.lua.globals().get::<i64>("renders").unwrap(), 1);

        assert!(dispatch_by_test_id(&mut runtime, &mut tree, "increment").unwrap());
        assert!(has_text(&tree, "count:1"));
        assert_eq!(runtime.lua.globals().get::<i64>("renders").unwrap(), 2);
        let store: Table = runtime.lua.globals().get("store").unwrap();
        let get_state: Function = store.get("get_state").unwrap();
        let state: Table = get_state.call(()).unwrap();
        assert_eq!(state.get::<i64>("noise").unwrap(), 1);
    }

    #[test]
    fn redux_store_supports_custom_selector_equality_and_listeners() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local ui = gpuix
                renders = 0
                notifications = 0
                local store = ui.create_store("parity", function(state, action)
                    return { count = state.count + action.amount }
                end, { count = 0 })
                local unsubscribe = store.subscribe(function()
                    notifications = notifications + 1
                end)

                return function()
                    renders = renders + 1
                    local selected = store.use_state(
                        function(state) return { parity = state.count % 2 } end,
                        function(previous, next) return previous.parity == next.parity end
                    )
                    return ui.div {
                        ui.text("parity:" .. selected.parity),
                        ui.div { testId = "add-two", onClick = function()
                            store.dispatch({ amount = 2 })
                        end },
                        ui.div { testId = "add-one", onClick = function()
                            store.dispatch({ amount = 1 })
                        end },
                        ui.div { testId = "unsubscribe", onClick = unsubscribe },
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        assert!(!dispatch_by_test_id(&mut runtime, &mut tree, "add-two").unwrap());
        assert_eq!(runtime.lua.globals().get::<i64>("renders").unwrap(), 1);
        assert_eq!(
            runtime.lua.globals().get::<i64>("notifications").unwrap(),
            1
        );
        assert!(dispatch_by_test_id(&mut runtime, &mut tree, "add-one").unwrap());
        assert!(has_text(&tree, "parity:1"));
        assert_eq!(runtime.lua.globals().get::<i64>("renders").unwrap(), 2);
        assert_eq!(
            runtime.lua.globals().get::<i64>("notifications").unwrap(),
            2
        );
        assert!(!dispatch_by_test_id(&mut runtime, &mut tree, "unsubscribe").unwrap());
        assert!(dispatch_by_test_id(&mut runtime, &mut tree, "add-one").unwrap());
        assert_eq!(
            runtime.lua.globals().get::<i64>("notifications").unwrap(),
            2
        );
    }

    #[test]
    fn redux_store_can_subscribe_to_the_entire_state() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local ui = gpuix
                local store = ui.create_store("whole", function(state, action)
                    return { count = state.count + 1 }
                end, { count = 0 })
                return function()
                    local state = store.use_state()
                    return ui.div {
                        ui.text("whole:" .. state.count),
                        ui.div { testId = "increment", onClick = function()
                            store.dispatch({})
                        end },
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        assert!(has_text(&tree, "whole:0"));
        assert!(dispatch_by_test_id(&mut runtime, &mut tree, "increment").unwrap());
        assert!(has_text(&tree, "whole:1"));
    }

    #[test]
    fn reload_preserves_named_store_state_and_replaces_its_reducer() {
        let app = TempLuaApp::new();
        let entry = app.write(
            "main.luax",
            r#"
                local ui = gpuix
                local store = ui.create_store("counter", function(state, action)
                    return { count = state.count + 1 }
                end, { count = 0 })
                notifications = notifications or 0
                store.subscribe(function() notifications = notifications + 1 end)
                return function()
                    local count = store.use_state(function(state) return state.count end)
                    return <div>
                        <text>one:{count}</text>
                        <div testId="increment" onClick={function() store.dispatch({}) end} />
                    </div>
                end
            "#,
        );
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_file(&entry, &mut tree).unwrap();
        dispatch_by_test_id(&mut runtime, &mut tree, "increment").unwrap();
        assert!(has_text(&tree, "one:1"));
        assert_eq!(
            runtime.lua.globals().get::<i64>("notifications").unwrap(),
            1
        );

        app.write(
            "main.luax",
            r#"
                local ui = gpuix
                local store = ui.create_store("counter", function(state, action)
                    return { count = state.count + 10 }
                end, { count = 100 })
                store.subscribe(function() notifications = notifications + 1 end)
                return function()
                    local count = store.use_state(function(state) return state.count end)
                    return <div>
                        <text>two:{count}</text>
                        <div testId="increment" onClick={function() store.dispatch({}) end} />
                    </div>
                end
            "#,
        );

        assert_eq!(
            runtime.reload_file(&entry, &mut tree).unwrap(),
            ReloadOutcome::PreservedState
        );
        assert!(has_text(&tree, "two:1"));
        dispatch_by_test_id(&mut runtime, &mut tree, "increment").unwrap();
        assert!(has_text(&tree, "two:11"));
        assert_eq!(
            runtime.lua.globals().get::<i64>("notifications").unwrap(),
            2
        );

        app.write(
            "main.luax",
            r#"
                local ui = gpuix
                local store = ui.create_store("counter", function(state, action)
                    return { count = state.count + 100 }
                end, { count = 1000 })
                store.subscribe(function() notifications = notifications + 100 end)
                return function()
                    error("failed replacement")
                end
            "#,
        );
        assert!(runtime.reload_file(&entry, &mut tree).is_err());
        dispatch_by_test_id(&mut runtime, &mut tree, "increment").unwrap();
        assert!(has_text(&tree, "two:21"));
        assert_eq!(
            runtime.lua.globals().get::<i64>("notifications").unwrap(),
            3
        );
    }

    #[test]
    fn swapping_different_hook_kinds_is_rejected() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local ui = gpuix
                return function()
                    local reversed, set_reversed = ui.use_state(false)
                    if reversed then
                        ui.use_ref(0)
                        ui.use_memo(function() return 0 end, {})
                    else
                        ui.use_memo(function() return 0 end, {})
                        ui.use_ref(0)
                    end
                    return ui.div { testId = "reverse", onClick = function() set_reversed(true) end }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        let error = dispatch_by_test_id(&mut runtime, &mut tree, "reverse").unwrap_err();
        assert!(error.contains("Lua hook order changed:"));
        assert!(error.contains("use_memo"));
        assert!(error.contains("use_ref(number)"));
    }

    #[test]
    fn source_relative_require_loads_lua_and_luax_modules_once() {
        let app = TempLuaApp::new();
        app.write(
            "data.lua",
            r#"
                module_loads = (module_loads or 0) + 1
                return { label = "Imported component" }
            "#,
        );
        app.write(
            "components/card.luax",
            r#"
                return function(props)
                    return <div testId="card"><text>{props.label}</text></div>
                end
            "#,
        );
        let entry = app.write(
            "main.luax",
            r#"
                local data = require("data")
                local same_data = require("data")
                local Card = require("components.card")
                assert(data == same_data)
                return function()
                    return <div><Card label={data.label} /></div>
                end
            "#,
        );
        let mut tree = RetainedTree::new();

        let runtime = LuaRuntime::load_file(&entry, &mut tree).unwrap();

        assert_eq!(runtime.lua.globals().get::<i64>("module_loads").unwrap(), 1);
        assert!(tree
            .elements
            .values()
            .any(|element| element.content.as_deref() == Some("Imported component")));
        assert!(tree
            .elements
            .values()
            .any(|element| element.test_id.as_deref() == Some("card")));
    }

    #[test]
    fn reload_reexecutes_modules_and_preserves_hook_state() {
        let app = TempLuaApp::new();
        app.write(
            "components/card.luax",
            r#"
                return function()
                    return <text>Version one</text>
                end
            "#,
        );
        let entry = app.write(
            "main.luax",
            r#"
                local Card = require("components.card")
                return function()
                    local count, set_count = gpuix.use_state(0)
                    return <div>
                        <div testId="increment" onClick={function() set_count(count + 1) end}>
                            <text>Count: {count}</text>
                        </div>
                        <Card />
                    </div>
                end
            "#,
        );
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_file(&entry, &mut tree).unwrap();
        let increment_id = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("increment"))
            .unwrap()
            .id;
        runtime
            .dispatch_event(
                EventPayload {
                    element_id: increment_id as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
                &mut tree,
            )
            .unwrap();

        app.write(
            "components/card.luax",
            r#"
                return function()
                    return <text>Version two</text>
                end
            "#,
        );
        app.write(
            "main.luax",
            r#"
                local Card = require("components.card")
                return function()
                    local count, set_count = gpuix.use_state(100)
                    return <div>
                        <div testId="increment" onClick={function() set_count(count + 1) end}>
                            <text>Count: {count}</text>
                        </div>
                        <Card />
                    </div>
                end
            "#,
        );
        let outcome = runtime.reload_file(&entry, &mut tree).unwrap();

        assert_eq!(outcome, ReloadOutcome::PreservedState);
        assert!(tree
            .elements
            .values()
            .any(|element| { element.content.as_deref() == Some("Count: 1") }));
        assert!(tree
            .elements
            .values()
            .any(|element| { element.content.as_deref() == Some("Version two") }));
        assert!(!tree
            .elements
            .values()
            .any(|element| { element.content.as_deref() == Some("Version one") }));
    }

    #[test]
    fn reload_refreshes_memo_callback_and_effect_closures() {
        let app = TempLuaApp::new();
        let entry = app.write(
            "main.lua",
            r#"
                effect_version = "none"
                cleanup_version = "none"
                return function()
                    local count, set_count = gpuix.use_state(0)
                    local label = gpuix.use_memo(function() return "one" end, {})
                    local read = gpuix.use_callback(function() return "one:" .. count end, {})
                    gpuix.use_effect(function()
                        effect_version = "one"
                        return function() cleanup_version = "one" end
                    end, {})
                    return gpuix.div {
                        gpuix.text(label .. ":" .. read()),
                        gpuix.div { testId = "increment", onClick = function() set_count(count + 1) end },
                    }
                end
            "#,
        );
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_file(&entry, &mut tree).unwrap();
        dispatch_by_test_id(&mut runtime, &mut tree, "increment").unwrap();
        assert!(has_text(&tree, "one:one:0"));

        app.write(
            "main.lua",
            r#"
                return function()
                    local count, set_count = gpuix.use_state(0)
                    local label = gpuix.use_memo(function() return "two" end, {})
                    local read = gpuix.use_callback(function() return "two:" .. count end, {})
                    gpuix.use_effect(function()
                        effect_version = "two"
                        return function() cleanup_version = "two" end
                    end, {})
                    return gpuix.div {
                        gpuix.text(label .. ":" .. read()),
                        gpuix.div { testId = "increment", onClick = function() set_count(count + 1) end },
                    }
                end
            "#,
        );

        let outcome = runtime.reload_file(&entry, &mut tree).unwrap();

        assert_eq!(outcome, ReloadOutcome::PreservedState);
        assert!(has_text(&tree, "two:two:1"));
        assert_eq!(
            runtime
                .lua
                .globals()
                .get::<String>("cleanup_version")
                .unwrap(),
            "one"
        );
        assert_eq!(
            runtime
                .lua
                .globals()
                .get::<String>("effect_version")
                .unwrap(),
            "two"
        );
    }

    #[test]
    fn reload_resets_state_when_the_hook_count_changes() {
        let app = TempLuaApp::new();
        let entry = app.write(
            "main.lua",
            r#"
                return function()
                    local count = gpuix.use_state(7)
                    return gpuix.div { gpuix.text("Count: " .. count) }
                end
            "#,
        );
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_file(&entry, &mut tree).unwrap();

        app.write(
            "main.lua",
            r#"
                return function()
                    local count = gpuix.use_state(100)
                    gpuix.use_state("new hook")
                    return gpuix.div { gpuix.text("Count: " .. count) }
                end
            "#,
        );
        let outcome = runtime.reload_file(&entry, &mut tree).unwrap();

        assert_eq!(outcome, ReloadOutcome::ResetState);
        assert!(tree
            .elements
            .values()
            .any(|element| { element.content.as_deref() == Some("Count: 100") }));
    }

    #[test]
    fn reload_resets_only_the_component_with_a_changed_hook_signature() {
        let app = TempLuaApp::new();
        let entry = app.write(
            "main.luax",
            r#"
                local function Left()
                    local value, set_value = gpuix.use_state(0)
                    return <div testId="left" onClick={function() set_value(value + 1) end}>
                        <text>left:{value}</text>
                    </div>
                end
                local function Right()
                    local value, set_value = gpuix.use_state(10)
                    return <div testId="right" onClick={function() set_value(value + 1) end}>
                        <text>right:{value}</text>
                    </div>
                end
                return function()
                    return <div><Left key="left" /><Right key="right" /></div>
                end
            "#,
        );
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_file(&entry, &mut tree).unwrap();
        dispatch_by_test_id(&mut runtime, &mut tree, "left").unwrap();
        dispatch_by_test_id(&mut runtime, &mut tree, "right").unwrap();

        app.write(
            "main.luax",
            r#"
                local function Left()
                    local value = gpuix.use_state("reset")
                    return <div testId="left"><text>left:{value}</text></div>
                end
                local function Right()
                    local value, set_value = gpuix.use_state(10)
                    return <div testId="right" onClick={function() set_value(value + 1) end}>
                        <text>right:{value}</text>
                    </div>
                end
                return function()
                    return <div><Left key="left" /><Right key="right" /></div>
                end
            "#,
        );
        let outcome = runtime.reload_file(&entry, &mut tree).unwrap();

        assert_eq!(outcome, ReloadOutcome::ResetState);
        assert!(has_text(&tree, "left:reset"));
        assert!(has_text(&tree, "right:11"));
    }

    #[test]
    fn luax_reload_preserves_hook_values_across_initializer_and_line_edits() {
        let app = TempLuaApp::new();
        let entry = app.write(
            "main.luax",
            r#"
                local ui = gpuix
                return function()
                    local count, set_count = ui.use_state(0)
                    return <div testId="increment" onClick={function() set_count(count + 1) end}>
                        <text>{"Count: " .. count}</text>
                    </div>
                end
            "#,
        );
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_file(&entry, &mut tree).unwrap();
        dispatch_by_test_id(&mut runtime, &mut tree, "increment").unwrap();

        app.write(
            "main.luax",
            r#"
                local ui = gpuix

                return function()

                    local count, set_count = ui.use_state(100)
                    return <div testId="increment" onClick={function() set_count(count + 1) end}>
                        <text>{"Updated: " .. count}</text>
                    </div>
                end
            "#,
        );
        let outcome = runtime.reload_file(&entry, &mut tree).unwrap();

        assert_eq!(outcome, ReloadOutcome::PreservedState);
        assert!(has_text(&tree, "Updated: 1"));
    }

    #[test]
    fn failed_reload_keeps_the_previous_render_function() {
        let app = TempLuaApp::new();
        let entry = app.write(
            "main.lua",
            r#"
                return function()
                    local count, set_count = gpuix.use_state(0)
                    return gpuix.div {
                        testId = "increment",
                        onClick = function() set_count(count + 1) end,
                        gpuix.text("Count: " .. count),
                    }
                end
            "#,
        );
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_file(&entry, &mut tree).unwrap();
        app.write("main.lua", "return function( this is not Lua");

        assert!(runtime.reload_file(&entry, &mut tree).is_err());
        let increment_id = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("increment"))
            .unwrap()
            .id;
        runtime
            .dispatch_event(
                EventPayload {
                    element_id: increment_id as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
                &mut tree,
            )
            .unwrap();
        assert!(tree
            .elements
            .values()
            .any(|element| { element.content.as_deref() == Some("Count: 1") }));
    }

    #[test]
    fn module_names_cannot_escape_the_entry_directory() {
        let root = Path::new("/tmp/app");
        assert!(module_candidates(root, "components.card").is_ok());
        assert!(module_candidates(root, "../secret").is_err());
        assert!(module_candidates(root, "components/card").is_err());
    }

    #[test]
    fn luax_components_update_through_native_event_handlers() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_luax(
            r#"
                local ui = gpuix

                local function Button(props)
                    return <div key="button" testId="button" onClick={props.onClick}>
                        <text>{props.label}</text>
                    </div>
                end

                return function()
                    local count, set_count = ui.use_state(0)
                    return <div testId="root">
                        <text>Count: {count}</text>
                        <Button
                            label="Increment"
                            onClick={function()
                                set_count(function(value) return value + 1 end)
                            end}
                        />
                    </div>
                end
            "#,
            &mut tree,
        )
        .unwrap();

        let button_id = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("button"))
            .unwrap()
            .id;
        assert!(runtime
            .dispatch_event(
                EventPayload {
                    element_id: button_id as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
                &mut tree,
            )
            .unwrap());
        assert!(tree
            .elements
            .values()
            .any(|element| element.content.as_deref() == Some("Count: 1")));
    }

    #[test]
    fn luax_store_hooks_accept_compiler_sites() {
        let mut tree = RetainedTree::new();
        LuaRuntime::load_luax(
            r#"
                local store = gpuix.create_store(
                    "counter",
                    function(state) return state end,
                    { count = 7 }
                )
                return function()
                    local count = store.use_state(function(state) return state.count end)
                    return <text>{"Count: " .. count}</text>
                end
            "#,
            &mut tree,
        )
        .unwrap();

        assert!(has_text(&tree, "Count: 7"));
    }

    #[test]
    fn keyed_component_reorders_preserve_each_components_state() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_luax(
            r#"
                local ui = gpuix

                local function Child(props)
                    local value, set_value = ui.use_state(props.initial)
                    return <div
                        testId={"child-" .. props.label}
                        onClick={function() set_value(value + 1) end}
                    >
                        <text>{props.label .. ":" .. value}</text>
                    </div>
                end

                return function()
                    local reversed, set_reversed = ui.use_state(false)
                    local children
                    if reversed then
                        children = {
                            <Child key="b" label="b" initial={10} />,
                            <Child key="a" label="a" initial={0} />,
                        }
                    else
                        children = {
                            <Child key="a" label="a" initial={0} />,
                            <Child key="b" label="b" initial={10} />,
                        }
                    end
                    return <div>
                        <div testId="reverse" onClick={function() set_reversed(not reversed) end} />
                        <div children={children} />
                    </div>
                end
            "#,
            &mut tree,
        )
        .unwrap();

        dispatch_by_test_id(&mut runtime, &mut tree, "child-a").unwrap();
        dispatch_by_test_id(&mut runtime, &mut tree, "reverse").unwrap();

        assert!(has_text(&tree, "a:1"));
        assert!(has_text(&tree, "b:10"));
    }

    #[test]
    fn remounting_a_keyed_component_starts_with_fresh_state() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local ui = gpuix
                local function Child()
                    local value, set_value = ui.use_state(0)
                    return ui.div {
                        testId = "child",
                        onClick = function() set_value(value + 1) end,
                        ui.text("child:" .. value),
                    }
                end
                return function()
                    local visible, set_visible = ui.use_state(true)
                    return ui.div {
                        ui.div {
                            testId = "toggle",
                            onClick = function() set_visible(not visible) end,
                        },
                        visible and ui.h(Child, { key = "child" }) or nil,
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        dispatch_by_test_id(&mut runtime, &mut tree, "child").unwrap();
        dispatch_by_test_id(&mut runtime, &mut tree, "toggle").unwrap();
        dispatch_by_test_id(&mut runtime, &mut tree, "toggle").unwrap();

        assert!(has_text(&tree, "child:0"));
        assert!(!has_text(&tree, "child:1"));
    }

    #[test]
    fn swapping_different_use_state_types_is_rejected() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local ui = gpuix
                return function()
                    local reversed, set_reversed = ui.use_state(false)
                    local first, second
                    if reversed then
                        first = ui.use_state("name")
                        second = ui.use_state(0)
                    else
                        first = ui.use_state(0)
                        second = ui.use_state("name")
                    end
                    return ui.div {
                        testId = "reverse",
                        onClick = function() set_reversed(not reversed) end,
                        ui.text(first .. ":" .. second),
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        let error = dispatch_by_test_id(&mut runtime, &mut tree, "reverse").unwrap_err();

        assert!(error.contains("use_state(number)"));
        assert!(error.contains("use_state(string)"));
        assert!(has_text(&tree, "0:name"));
    }

    #[test]
    fn luax_rejects_swapping_same_type_hooks() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_luax(
            r#"
                local ui = gpuix
                return function()
                    local reversed, set_reversed = ui.use_state(false)
                    local first, second
                    if reversed then
                        second = ui.use_state(0)
                        first = ui.use_state(0)
                    else
                        first = ui.use_state(0)
                        second = ui.use_state(0)
                    end
                    return <div
                        testId="reverse"
                        onClick={function() set_reversed(not reversed) end}
                    >
                        <text>{first .. ":" .. second}</text>
                    </div>
                end
            "#,
            &mut tree,
        )
        .unwrap();

        let error = dispatch_by_test_id(&mut runtime, &mut tree, "reverse").unwrap_err();

        assert!(error.contains("Lua hook order changed:"));
        assert!(error.contains("LuaX site"));
        assert!(has_text(&tree, "0:0"));
    }

    #[test]
    fn conditional_hooks_reject_the_render_and_preserve_the_previous_tree() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local ui = gpuix
                return function()
                    local enabled, set_enabled = ui.use_state(true)
                    if enabled then
                        ui.use_state("conditional")
                    end
                    return ui.div {
                        testId = "root",
                        onClick = function() set_enabled(function(value) return not value end) end,
                        ui.text(enabled and "enabled" or "disabled"),
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        let root_id = tree.root_id.unwrap();
        let before_ids = tree.elements.keys().copied().collect::<HashSet<_>>();
        let error = runtime
            .dispatch_event(
                EventPayload {
                    element_id: root_id as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
                &mut tree,
            )
            .unwrap_err();

        assert!(error.contains("rendered 1 hooks"));
        assert!(error.contains("previous render used 2"));
        assert_eq!(tree.root_id, Some(root_id));
        assert_eq!(
            tree.elements.keys().copied().collect::<HashSet<_>>(),
            before_ids
        );
        assert!(tree
            .elements
            .values()
            .any(|element| element.content.as_deref() == Some("enabled")));

        assert!(runtime
            .dispatch_event(
                EventPayload {
                    element_id: root_id as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
                &mut tree,
            )
            .unwrap());
        assert!(tree
            .elements
            .values()
            .any(|element| element.content.as_deref() == Some("enabled")));
    }

    #[test]
    fn adding_a_conditional_hook_does_not_leave_an_extra_state_slot() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local ui = gpuix
                return function()
                    local enabled, set_enabled = ui.use_state(false)
                    if enabled then
                        ui.use_state("conditional")
                    end
                    return ui.div {
                        onClick = function() set_enabled(true) end,
                        ui.text("root"),
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();
        let root_id = tree.root_id.unwrap();

        let error = runtime
            .dispatch_event(
                EventPayload {
                    element_id: root_id as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
                &mut tree,
            )
            .unwrap_err();

        assert!(error.contains("rendered 2 hooks"));
        assert!(error.contains("previous render used 1"));
        assert_eq!(runtime.hooks.lock().unwrap().slot_count(), 1);
    }

    #[test]
    fn memo_hits_reserve_component_positions_for_later_siblings() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local ui = gpuix
                local function Child(props)
                    local value, set_value = ui.use_state(props.initial)
                    return ui.div {
                        testId = props.testId,
                        onClick = function() set_value(value + 1) end,
                        ui.text(props.testId .. ":" .. value),
                    }
                end
                return function()
                    local cached = ui.memo("cached", true, function()
                        return ui.h(Child, { testId = "cached", initial = 0 })
                    end)
                    local later = ui.h(Child, { testId = "later", initial = 10 })
                    return ui.div { cached, later }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        dispatch_by_test_id(&mut runtime, &mut tree, "cached").unwrap();
        dispatch_by_test_id(&mut runtime, &mut tree, "later").unwrap();

        assert!(has_text(&tree, "cached:1"));
        assert!(has_text(&tree, "later:11"));
    }

    #[test]
    fn memo_reuses_unchanged_host_subtrees() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local ui = gpuix
                return function()
                    local selected, set_selected = ui.use_state(1)
                    local rows = {}
                    for index = 1, 3 do
                        local active = selected == index
                        rows[index] = ui.memo(index, active, function()
                            return ui.div {
                                key = index,
                                testId = "row-" .. index,
                                ui.text(active and "active" or "idle"),
                            }
                        end)
                    end
                    return ui.div {
                        onClick = function() set_selected(2) end,
                        children = rows,
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        let row_three = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("row-3"))
            .unwrap();
        let row_three_id = row_three.id;
        let row_three_revision = row_three.subtree_revision;
        runtime
            .dispatch_event(
                EventPayload {
                    element_id: tree.root_id.unwrap() as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
                &mut tree,
            )
            .unwrap();

        let row_three = &tree.elements[&row_three_id];
        assert_eq!(row_three.subtree_revision, row_three_revision);
        assert!(tree
            .elements
            .values()
            .any(|element| element.content.as_deref() == Some("active")));
    }

    #[test]
    fn memo_batch_reuses_unchanged_host_subtrees() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local ui = gpuix
                local keys = { 1, 2, 3 }
                return function()
                    local selected, set_selected = ui.use_state(1)
                    local dependencies = {}
                    for index = 1, #keys do
                        dependencies[index] = selected == index
                    end
                    local rows = ui.memo_batch(keys, dependencies, function(index, active)
                        return ui.div {
                            key = index,
                            testId = "row-" .. index,
                            ui.text(active and "active" or "idle"),
                        }
                    end)
                    return ui.div {
                        onClick = function() set_selected(2) end,
                        children = rows,
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        let row_three = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("row-3"))
            .unwrap();
        let row_three_id = row_three.id;
        let row_three_revision = row_three.subtree_revision;
        runtime
            .dispatch_event(
                EventPayload {
                    element_id: tree.root_id.unwrap() as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
                &mut tree,
            )
            .unwrap();

        assert_eq!(
            tree.elements[&row_three_id].subtree_revision,
            row_three_revision
        );
        assert!(tree
            .elements
            .values()
            .any(|element| element.content.as_deref() == Some("active")));
    }

    #[test]
    fn memo_batch_child_lists_flatten_and_change_length() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local ui = gpuix
                return function()
                    local count, set_count = ui.use_state(2)
                    local keys = {}
                    local dependencies = {}
                    for index = 1, count do
                        keys[index] = index
                        dependencies[index] = false
                    end
                    local rows = ui.memo_batch(keys, dependencies, function(index)
                        return ui.div { key = index, testId = "row-" .. index }
                    end)
                    assert(rows < 0)
                    return ui.div {
                        onClick = function() set_count(3) end,
                        ui.text("before"),
                        rows,
                        ui.text("after"),
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        let root_id = tree.root_id.unwrap();
        assert_eq!(tree.elements[&root_id].children.len(), 4);
        runtime
            .dispatch_event(
                EventPayload {
                    element_id: root_id as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
                &mut tree,
            )
            .unwrap();

        let children = &tree.elements[&root_id].children;
        assert_eq!(children.len(), 5);
        assert_eq!(
            tree.elements[&children[0]].content.as_deref(),
            Some("before")
        );
        assert_eq!(
            tree.elements[children.last().unwrap()].content.as_deref(),
            Some("after")
        );
        assert!(tree
            .elements
            .values()
            .any(|element| element.test_id.as_deref() == Some("row-3")));
    }

    #[test]
    fn memo_batch_child_lists_keep_parent_key_validation() {
        let mut tree = RetainedTree::new();
        let error = LuaRuntime::load(
            r#"
                local ui = gpuix
                return function()
                    local rows = ui.memo_batch({ 1, 2 }, { false, false }, function(index)
                        return ui.div { key = "duplicate" }
                    end)
                    return ui.div { children = rows }
                end
            "#,
            &mut tree,
        )
        .err()
        .unwrap();

        assert!(error.contains("duplicate Lua child key"));
    }

    #[test]
    fn memo_batch_rejects_sparse_tables() {
        let mut tree = RetainedTree::new();
        let sparse_keys = LuaRuntime::load(
            r#"
                local ui = gpuix
                return function()
                    local keys = { 1, 2, 3 }
                    keys[2] = nil
                    return ui.div {
                        children = ui.memo_batch(keys, { false, false, false }, function(index)
                            return ui.div { key = index }
                        end),
                    }
                end
            "#,
            &mut tree,
        )
        .err()
        .unwrap();
        assert!(sparse_keys.contains("must be dense tables"));

        let mut tree = RetainedTree::new();
        let sparse_dependencies = LuaRuntime::load(
            r#"
                local ui = gpuix
                return function()
                    local dependencies = { false, false, false }
                    dependencies[2] = nil
                    return ui.div {
                        children = ui.memo_batch({ 1, 2, 3 }, dependencies, function(index)
                            return ui.div { key = index }
                        end),
                    }
                end
            "#,
            &mut tree,
        )
        .err()
        .unwrap();
        assert!(sparse_dependencies.contains("must be dense tables"));
    }

    #[test]
    fn host_nodes_cannot_repeat_under_one_parent() {
        let mut tree = RetainedTree::new();
        let error = LuaRuntime::load(
            r#"
                local ui = gpuix
                return function()
                    local child = ui.div {}
                    return ui.div { child, child }
                end
            "#,
            &mut tree,
        )
        .err()
        .unwrap();

        assert!(error.contains("same host node cannot appear twice"));
    }

    #[test]
    fn host_nodes_cannot_belong_to_multiple_parents() {
        let mut tree = RetainedTree::new();
        let error = LuaRuntime::load(
            r#"
                local ui = gpuix
                return function()
                    local child = ui.div {}
                    local first = ui.div { child }
                    local second = ui.div { child }
                    return ui.div { first, second }
                end
            "#,
            &mut tree,
        )
        .err()
        .unwrap();

        assert!(error.contains("more than one parent"));
    }

    #[test]
    fn memo_keys_preserve_lua_types() {
        let mut tree = RetainedTree::new();
        LuaRuntime::load(
            r#"
                local ui = gpuix
                return function()
                    return ui.div {
                        ui.memo(1, true, function()
                            return ui.div { key = "number" }
                        end),
                        ui.memo("1", true, function()
                            return ui.div { key = "string" }
                        end),
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        let root = &tree.elements[&tree.root_id.unwrap()];
        assert_eq!(root.children.len(), 2);
    }

    #[test]
    fn memo_table_dependencies_ignore_insertion_order() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local ui = gpuix
                local builds = 0
                return function()
                    local reversed, set_reversed = ui.use_state(false)
                    local dependencies = {}
                    if reversed then
                        dependencies.second = 2
                        dependencies.first = true
                    else
                        dependencies.first = true
                        dependencies.second = 2
                    end
                    return ui.div {
                        onClick = function() set_reversed(true) end,
                        ui.memo("row", dependencies, function()
                            builds = builds + 1
                            return ui.div {
                                key = "row",
                                testId = "row",
                                ui.text(builds),
                            }
                        end),
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        runtime
            .dispatch_event(
                EventPayload {
                    element_id: tree.root_id.unwrap() as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
                &mut tree,
            )
            .unwrap();

        assert!(tree
            .elements
            .values()
            .any(|element| element.content.as_deref() == Some("1")));
        assert!(!tree
            .elements
            .values()
            .any(|element| element.content.as_deref() == Some("2")));
    }

    #[test]
    fn keyed_reorders_preserve_ids_when_the_ordered_fast_path_misses() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load(
            r#"
                local ui = gpuix
                return function()
                    local reversed, set_reversed = ui.use_state(false)
                    local first = ui.div { key = "first", testId = "first" }
                    local second = ui.div { key = "second", testId = "second" }
                    return ui.div {
                        onClick = function() set_reversed(true) end,
                        children = reversed and { second, first } or { first, second },
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        let root_id = tree.root_id.unwrap();
        let first_id = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("first"))
            .unwrap()
            .id;
        let second_id = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("second"))
            .unwrap()
            .id;

        runtime
            .dispatch_event(
                EventPayload {
                    element_id: root_id as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
                &mut tree,
            )
            .unwrap();

        assert_eq!(tree.elements[&root_id].children, vec![second_id, first_id]);
    }

    #[test]
    fn native_style_handles_can_be_reused_without_table_conversion() {
        let mut tree = RetainedTree::new();
        LuaRuntime::load(
            r#"
                local ui = gpuix
                local row_style = ui.style { display = "flex", gap = 8 }
                return function()
                    return ui.div {
                        ui.div { style = row_style, ui.text("first") },
                        ui.div { style = row_style, ui.text("second") },
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        let root = &tree.elements[&tree.root_id.unwrap()];
        let first = tree.elements[&root.children[0]].style.as_ref().unwrap();
        let second = tree.elements[&root.children[1]].style.as_ref().unwrap();
        assert!(Arc::ptr_eq(first, second));
    }

    #[test]
    fn text_values_create_one_native_node_and_handles_are_integers() {
        let mut tree = RetainedTree::new();
        LuaRuntime::load(
            r#"
                local ui = gpuix
                return function()
                    local first = ui.text("first")
                    assert(type(first) == "number")
                    assert(math.type(first) == "integer")
                    return ui.div {
                        first,
                        ui.text { content = 42 },
                    }
                end
            "#,
            &mut tree,
        )
        .unwrap();

        let root = &tree.elements[&tree.root_id.unwrap()];
        assert_eq!(tree.elements.len(), 3);
        assert_eq!(root.children.len(), 2);
        assert_eq!(
            tree.elements[&root.children[0]].content.as_deref(),
            Some("first")
        );
        assert_eq!(
            tree.elements[&root.children[1]].content.as_deref(),
            Some("42")
        );
    }

    #[test]
    fn bundled_luax_components_load_without_a_source_directory() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_luax(
            r#"
                local Button = require("gpuix.button")
                local Checkbox = require("gpuix.checkbox")
                return function()
                    local checked, set_checked = gpuix.use_state(false)
                    return <div>
                        <Button key="bundled-button" testId="bundled-button">
                            <text>button</text>
                        </Button>
                        <Checkbox
                            key="bundled-checkbox"
                            testId="bundled-checkbox"
                            checked={checked}
                            defaultChecked
                            label={checked and "on" or "off"}
                            onCheckedChange={set_checked}
                        />
                    </div>
                end
            "#,
            &mut tree,
        )
        .unwrap();

        let button = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("bundled-button"))
            .unwrap();
        assert_eq!(
            button.custom_props.get("tabIndex"),
            Some(&serde_json::json!(0))
        );
        assert!(has_text(&tree, "off"));
        assert!(!has_text(&tree, "✓"));
        assert!(
            dispatch_by_test_id(&mut runtime, &mut tree, "bundled-checkbox-indicator").unwrap()
        );
        assert!(has_text(&tree, "on"));
        assert!(has_text(&tree, "✓"));
    }

    #[test]
    fn bundled_drawer_resizes_with_pointer_and_keyboard() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_luax(
            r#"
                local Drawer = require("gpuix.drawer")
                return function()
                    return <div style={{ width = 800, height = 600 }}>
                        <Drawer
                            testId="drawer"
                            side="left"
                            defaultSize={200}
                            minSize={120}
                            maxSize={300}
                            renderContent={function(state)
                                return <text>{string.format("Size: %.0f", state.size)}</text>
                            end}
                        />
                    </div>
                end
            "#,
            &mut tree,
        )
        .unwrap();

        dispatch_event_by_test_id(
            &mut runtime,
            &mut tree,
            "drawer-resize-handle",
            EventPayload {
                event_type: "mouseDown".to_string(),
                x: Some(200.0),
                button: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
        dispatch_event_by_test_id(
            &mut runtime,
            &mut tree,
            "drawer-resize-handle",
            EventPayload {
                event_type: "mouseMove".to_string(),
                x: Some(260.0),
                pressed_button: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(has_text(&tree, "Size: 260"));

        dispatch_event_by_test_id(
            &mut runtime,
            &mut tree,
            "drawer-resize-handle",
            EventPayload {
                event_type: "keyDown".to_string(),
                key: Some("right".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(has_text(&tree, "Size: 268"));

        dispatch_event_by_test_id(
            &mut runtime,
            &mut tree,
            "drawer-resize-handle",
            EventPayload {
                event_type: "mouseMove".to_string(),
                x: Some(1000.0),
                pressed_button: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(has_text(&tree, "Size: 300"));

        dispatch_event_by_test_id(
            &mut runtime,
            &mut tree,
            "drawer-resize-handle",
            EventPayload {
                event_type: "mouseUp".to_string(),
                click_count: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(has_text(&tree, "Size: 200"));
    }

    #[test]
    fn bundled_dock_layout_toggles_and_moves_panels() {
        let mut tree = RetainedTree::new();
        let mut runtime = LuaRuntime::load_luax(
            r#"
                local DockLayout = require("gpuix.dock_layout")
                local function ActivityPanel(props)
                    local visits, set_visits = gpuix.use_state(0)
                    return <div>
                        <text>{"Activity " .. props.position}</text>
                        <div testId="activity-increment" onClick={function()
                            set_visits(visits + 1)
                        end}><text>{"Visits " .. visits}</text></div>
                    </div>
                end
                return function()
                    local panels = {{
                        id = "activity",
                        label = "Activity",
                        position = "right",
                        defaultOpen = true,
                        defaultSize = 240,
                        render = function(state)
                            return <ActivityPanel key="activity-content" position={state.position} />
                        end,
                    }}
                    return <DockLayout testId="dock" panels={panels}>
                        <text>Content</text>
                    </DockLayout>
                end
            "#,
            &mut tree,
        )
        .unwrap();

        assert!(has_test_id(&tree, "dock-panel-activity"));
        assert!(has_test_id(&tree, "dock-button-activity"));
        assert!(has_text(&tree, "Activity right"));

        dispatch_by_test_id(&mut runtime, &mut tree, "activity-increment").unwrap();
        assert!(has_text(&tree, "Visits 1"));
        dispatch_by_test_id(&mut runtime, &mut tree, "dock-button-activity").unwrap();
        assert!(!has_test_id(&tree, "dock-panel-activity"));
        dispatch_by_test_id(&mut runtime, &mut tree, "dock-button-activity").unwrap();
        assert!(has_text(&tree, "Visits 1"));

        dispatch_event_by_test_id(
            &mut runtime,
            &mut tree,
            "dock-button-activity",
            EventPayload {
                event_type: "auxClick".to_string(),
                x: Some(700.0),
                y: Some(500.0),
                is_right_click: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(has_test_id(&tree, "dock-button-menu"));
        dispatch_by_test_id(&mut runtime, &mut tree, "dock-button-activity-dock-left").unwrap();
        assert!(has_text(&tree, "Activity left"));
        assert!(has_text(&tree, "Visits 1"));
        assert!(!has_test_id(&tree, "dock-button-menu"));
    }

    #[test]
    fn primitive_children_require_an_explicit_text_constructor() {
        let mut tree = RetainedTree::new();
        let error = LuaRuntime::load(
            r#"
                local ui = gpuix
                return function()
                    return ui.div { "plain text" }
                end
            "#,
            &mut tree,
        )
        .err()
        .unwrap();

        assert!(error.contains("wrap text with gpuix.text(value)"));
    }

    #[test]
    fn application_windows_share_stores_and_keep_local_hooks_isolated() {
        let mut runtime = LuaRuntime::load_source_unrendered(
            r#"
                local ui = gpuix
                local app = ui.create_app()
                local store = ui.create_store("counter", function(state, action)
                    if action.type == "increment" then
                        return { count = state.count + 1 }
                    end
                    return state
                end, { count = 0 })

                ui.define_window(app, {
                    id = "main",
                    title = "Main",
                    render = function()
                        local local_count, set_local_count = ui.use_state(0)
                        local count = store.use_state(function(state) return state.count end)
                        return ui.div {
                            testId = "main-button",
                            onClick = function()
                                set_local_count(local_count + 1)
                                store.dispatch({ type = "increment" })
                            end,
                            ui.text("main " .. local_count .. " shared " .. count),
                        }
                    end,
                })
                ui.define_window(app, {
                    id = "inspector",
                    title = "Inspector",
                    open = false,
                    render = function()
                        local local_count = ui.use_state(0)
                        local count = store.use_state(function(state) return state.count end)
                        return ui.div { ui.text("inspector " .. local_count .. " shared " .. count) }
                    end,
                })
                return app
            "#,
            None,
            false,
        )
        .unwrap();
        let mut main = RetainedTree::new();
        let mut inspector = RetainedTree::new();
        runtime.mount_window("main", &mut main).unwrap();
        runtime.mount_window("inspector", &mut inspector).unwrap();
        assert!(has_text(&main, "main 0 shared 0"));
        assert!(has_text(&inspector, "inspector 0 shared 0"));

        let element_id = main
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("main-button"))
            .unwrap()
            .id;
        let dirty = runtime
            .dispatch_window_event(
                "main",
                EventPayload {
                    element_id: element_id as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(dirty.len(), 2);
        assert!(dirty.iter().any(|root| root == "main"));
        assert!(dirty.iter().any(|root| root == "inspector"));
        runtime.mount_window("main", &mut main).unwrap();
        runtime.mount_window("inspector", &mut inspector).unwrap();
        assert!(has_text(&main, "main 1 shared 1"));
        assert!(has_text(&inspector, "inspector 0 shared 1"));

        runtime.unmount_window("main");
        main = RetainedTree::new();
        runtime.mount_window("main", &mut main).unwrap();
        assert!(has_text(&main, "main 0 shared 1"));
        assert!(has_text(&inspector, "inspector 0 shared 1"));
    }

    #[test]
    fn application_window_commands_use_explicit_app_handles() {
        let mut runtime = LuaRuntime::load_source_unrendered(
            r#"
                local ui = gpuix
                local app = ui.create_app()
                ui.define_window(app, {
                    id = "main",
                    render = function()
                        return ui.div {
                            testId = "open",
                            onClick = function() ui.open_window(app, "inspector") end,
                        }
                    end,
                })
                ui.define_window(app, {
                    id = "inspector",
                    open = false,
                    render = function() return ui.div {} end,
                })
                return app
            "#,
            None,
            false,
        )
        .unwrap();
        let mut main = RetainedTree::new();
        runtime.mount_window("main", &mut main).unwrap();
        let element_id = main
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("open"))
            .unwrap()
            .id;
        runtime
            .dispatch_window_event(
                "main",
                EventPayload {
                    element_id: element_id as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            runtime.take_app_commands(),
            vec![LuaAppCommand::Open("inspector".to_string())]
        );
    }

    #[test]
    fn application_window_reposition_option_defaults_off() {
        let runtime = LuaRuntime::load_source_unrendered(
            r#"
                local ui = gpuix
                local app = ui.create_app()
                ui.define_window(app, {
                    id = "main",
                    render = function() return ui.div {} end,
                })
                ui.define_window(app, {
                    id = "centered",
                    reposition = true,
                    render = function() return ui.div {} end,
                })
                return app
            "#,
            None,
            false,
        )
        .unwrap();
        let windows = runtime.window_options();
        assert!(!windows[0].reposition);
        assert!(windows[1].reposition);
    }

    #[test]
    fn focus_requests_follow_the_host_handle_to_its_window() {
        let mut runtime = LuaRuntime::load_source_unrendered(
            r#"
                local ui = gpuix
                local app = ui.create_app()
                local inspector_target = nil
                ui.define_window(app, {
                    id = "main",
                    render = function()
                        return ui.div {
                            testId = "focus-inspector",
                            onClick = function() ui.focus(inspector_target) end,
                        }
                    end,
                })
                ui.define_window(app, {
                    id = "inspector",
                    open = false,
                    render = function()
                        inspector_target = ui.div { testId = "inspector-target", tabIndex = 0 }
                        return inspector_target
                    end,
                })
                return app
            "#,
            None,
            false,
        )
        .unwrap();
        let mut main = RetainedTree::new();
        let mut inspector = RetainedTree::new();
        runtime.mount_window("main", &mut main).unwrap();
        runtime.mount_window("inspector", &mut inspector).unwrap();
        let element_id = main
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("focus-inspector"))
            .unwrap()
            .id;
        runtime
            .dispatch_window_event(
                "main",
                EventPayload {
                    element_id: element_id as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
            )
            .unwrap();

        let request = runtime.take_focus_request().unwrap();
        assert_eq!(request.root_id, "inspector");
        assert_eq!(
            inspector.elements[&request.element_id].test_id.as_deref(),
            Some("inspector-target")
        );
    }

    #[test]
    fn window_open_hook_reacts_to_open_and_unmount() {
        let mut runtime = LuaRuntime::load_source_unrendered(
            r#"
                local ui = gpuix
                local app = ui.create_app()
                ui.define_window(app, {
                    id = "main",
                    render = function()
                        local inspector_open = ui.use_window_open(app, "inspector")
                        return ui.div {
                            ui.text(inspector_open and "inspector open" or "inspector closed"),
                        }
                    end,
                })
                ui.define_window(app, {
                    id = "inspector",
                    open = false,
                    render = function() return ui.div {} end,
                })
                return app
            "#,
            None,
            false,
        )
        .unwrap();
        let mut main = RetainedTree::new();
        let mut inspector = RetainedTree::new();
        runtime.mount_window("main", &mut main).unwrap();
        assert!(has_text(&main, "inspector closed"));

        runtime.set_window_open("inspector", true);
        assert_eq!(runtime.dirty_window_ids(), vec!["main"]);
        runtime.mount_window("main", &mut main).unwrap();
        runtime.mount_window("inspector", &mut inspector).unwrap();
        assert!(has_text(&main, "inspector open"));

        runtime.unmount_window("inspector");
        assert_eq!(runtime.dirty_window_ids(), vec!["main"]);
        runtime.mount_window("main", &mut main).unwrap();
        assert!(has_text(&main, "inspector closed"));
    }

    #[test]
    fn application_reload_preserves_each_window_and_shared_store_state() {
        let app = TempLuaApp::new();
        let initial = app.write(
            "main.lua",
            r#"
                local ui = gpuix
                local app = ui.create_app()
                local store = ui.create_store("reload-counter", function(state, action)
                    if action.type == "increment" then
                        return { count = state.count + 1 }
                    end
                    return state
                end, { count = 0 })

                ui.define_window(app, {
                    id = "main",
                    render = function()
                        local local_count, set_local_count = ui.use_state(0)
                        local count = store.use_state(function(state) return state.count end)
                        return ui.div {
                            testId = "increment",
                            onClick = function()
                                set_local_count(local_count + 1)
                                store.dispatch({ type = "increment" })
                            end,
                            ui.text("initial main " .. local_count .. " shared " .. count),
                        }
                    end,
                })
                ui.define_window(app, {
                    id = "inspector",
                    open = false,
                    render = function()
                        local local_count = ui.use_state(0)
                        local count = store.use_state(function(state) return state.count end)
                        return ui.div {
                            ui.text("initial inspector " .. local_count .. " shared " .. count),
                        }
                    end,
                })
                return app
            "#,
        );
        let mut runtime = LuaRuntime::load_application_file(&initial).unwrap();
        let mut main = RetainedTree::new();
        let mut inspector = RetainedTree::new();
        runtime.mount_window("main", &mut main).unwrap();
        runtime.mount_window("inspector", &mut inspector).unwrap();

        let element_id = main
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some("increment"))
            .unwrap()
            .id;
        runtime
            .dispatch_window_event(
                "main",
                EventPayload {
                    element_id: element_id as f64,
                    event_type: "click".to_string(),
                    ..Default::default()
                },
            )
            .unwrap();
        runtime.mount_window("main", &mut main).unwrap();
        runtime.mount_window("inspector", &mut inspector).unwrap();

        app.write(
            "main.lua",
            r#"
                local ui = gpuix
                local app = ui.create_app()
                local store = ui.create_store("reload-counter", function(state, action)
                    if action.type == "increment" then
                        return { count = state.count + 1 }
                    end
                    return state
                end, { count = 0 })

                ui.define_window(app, {
                    id = "main",
                    render = function()
                        local local_count = ui.use_state(0)
                        local count = store.use_state(function(state) return state.count end)
                        return ui.div {
                            ui.text("reloaded main " .. local_count .. " shared " .. count),
                        }
                    end,
                })
                ui.define_window(app, {
                    id = "inspector",
                    open = false,
                    render = function()
                        local local_count = ui.use_state(0)
                        local count = store.use_state(function(state) return state.count end)
                        return ui.div {
                            ui.text("reloaded inspector " .. local_count .. " shared " .. count),
                        }
                    end,
                })
                return app
            "#,
        );

        let reload = runtime.reload_application_file(&initial).unwrap();
        assert!(reload.removed.is_empty());
        assert_eq!(
            reload
                .windows
                .iter()
                .map(|window| window.id.as_str())
                .collect::<Vec<_>>(),
            vec!["main", "inspector"]
        );
        assert_eq!(
            runtime.refresh_window("main", &mut main).unwrap(),
            ReloadOutcome::PreservedState
        );
        assert_eq!(
            runtime.refresh_window("inspector", &mut inspector).unwrap(),
            ReloadOutcome::PreservedState
        );
        assert!(has_text(&main, "reloaded main 1 shared 1"));
        assert!(has_text(&inspector, "reloaded inspector 0 shared 1"));
    }

    fn dispatch_by_test_id(
        runtime: &mut LuaRuntime,
        tree: &mut RetainedTree,
        test_id: &str,
    ) -> Result<bool, String> {
        dispatch_event_by_test_id(
            runtime,
            tree,
            test_id,
            EventPayload {
                event_type: "click".to_string(),
                ..Default::default()
            },
        )
    }

    fn dispatch_event_by_test_id(
        runtime: &mut LuaRuntime,
        tree: &mut RetainedTree,
        test_id: &str,
        mut payload: EventPayload,
    ) -> Result<bool, String> {
        payload.element_id = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some(test_id))
            .unwrap()
            .id as f64;
        runtime.dispatch_event(payload, tree)
    }

    fn has_text(tree: &RetainedTree, content: &str) -> bool {
        tree.elements
            .values()
            .any(|element| element.content.as_deref() == Some(content))
    }

    fn has_test_id(tree: &RetainedTree, test_id: &str) -> bool {
        tree.elements
            .values()
            .any(|element| element.test_id.as_deref() == Some(test_id))
    }
}
