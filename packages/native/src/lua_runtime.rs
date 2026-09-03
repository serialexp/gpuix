use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mlua::{Function, Lua, LuaSerdeExt, RegistryKey, Table, UserData, Value};

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

const BUILTIN_LUA_MODULES: &[(&str, &str, bool)] = &[
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct ComponentId(Vec<ComponentSlot>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HookKind {
    State,
    Reducer,
    Ref,
    Memo,
    Callback,
    Effect,
    Store,
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
    dirty: bool,
}

struct HookStore {
    components: HashMap<ComponentId, ComponentHooks>,
    frames: Vec<HookFrame>,
    seen: HashSet<ComponentId>,
    visited: Vec<ComponentId>,
    mismatch: Option<ComponentId>,
    pending_effects: Vec<PendingEffect>,
    refreshing: bool,
    dirty: bool,
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
            refreshing: false,
            dirty: false,
        }
    }

    fn snapshot(&self) -> HookSnapshot {
        HookSnapshot {
            components: self.components.clone(),
            dirty: self.dirty,
        }
    }

    fn restore(&mut self, snapshot: HookSnapshot) {
        self.components = snapshot.components;
        self.frames.clear();
        self.seen.clear();
        self.visited.clear();
        self.mismatch = None;
        self.pending_effects.clear();
        self.refreshing = false;
        self.dirty = snapshot.dirty;
    }

    fn rollback(&mut self, snapshot: HookSnapshot) {
        let mismatch = self.mismatch.take();
        self.restore(snapshot);
        self.mismatch = mismatch;
        self.dirty = false;
    }

    fn begin_render(&mut self, refreshing: bool) {
        self.frames.clear();
        self.seen.clear();
        self.visited.clear();
        self.mismatch = None;
        self.pending_effects.clear();
        self.refreshing = refreshing;
        self.dirty = false;
        let root = ComponentId::default();
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
        id.0.push(slot);
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
        self.dirty = true;
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
        if !changed_components.is_empty() {
            self.dirty = true;
        }
        changed_components
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
        self.dirty = false;
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
        let removed = self
            .components
            .extract_if(|component, _| !self.seen.contains(component))
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
    entries: HashMap<MemoKey, MemoEntry>,
    seen: HashSet<MemoKey>,
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

impl LuaNode {
    fn identity_matches(&self, element_type: &str, key: Option<&str>) -> bool {
        self.element_type == element_type && self.key.as_deref() == key
    }
}

pub(crate) struct LuaRuntime {
    lua: Lua,
    render: Function,
    hooks: Arc<Mutex<HookStore>>,
    stores: Arc<Mutex<StoreRegistry>>,
    memo: Arc<Mutex<MemoStore>>,
    arena: Arc<Mutex<RenderArena>>,
    host_handles: Arc<Mutex<HostHandleMap>>,
    focus_request: Arc<Mutex<Option<u64>>>,
    root: Option<LuaNode>,
    handlers: HashMap<(u64, String), Function>,
    loaded_modules: Arc<Mutex<HashSet<String>>>,
    next_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReloadOutcome {
    PreservedState,
    ResetState,
}

struct ModuleSnapshot {
    values: Vec<(String, Value)>,
}

impl LuaRuntime {
    pub(crate) fn load(source: &str, tree: &mut RetainedTree) -> Result<Self, String> {
        Self::load_source(source, None, false, tree)
    }

    pub(crate) fn load_luax(source: &str, tree: &mut RetainedTree) -> Result<Self, String> {
        Self::load_source(source, None, true, tree)
    }

    pub(crate) fn load_file(path: &Path, tree: &mut RetainedTree) -> Result<Self, String> {
        let source = std::fs::read_to_string(path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        let luax = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("luax"));
        Self::load_source(&source, Some(path), luax, tree)
    }

    fn load_source(
        source: &str,
        path: Option<&Path>,
        luax: bool,
        tree: &mut RetainedTree,
    ) -> Result<Self, String> {
        let lua = Lua::new();
        let hooks = Arc::new(Mutex::new(HookStore::new()));
        let stores = Arc::new(Mutex::new(StoreRegistry::default()));
        let memo = Arc::new(Mutex::new(MemoStore::default()));
        let arena = Arc::new(Mutex::new(RenderArena::default()));
        let host_handles = Arc::new(Mutex::new(HostHandleMap::new()));
        let focus_request = Arc::new(Mutex::new(None));
        let styles = Arc::new(Mutex::new(StyleCache::new()));
        let loaded_modules = Arc::new(Mutex::new(HashSet::new()));
        install_api(
            &lua,
            hooks.clone(),
            stores.clone(),
            memo.clone(),
            arena.clone(),
            host_handles.clone(),
            focus_request.clone(),
            styles,
        )
        .map_err(lua_error)?;
        install_builtin_modules(&lua).map_err(lua_error)?;
        if let Some(root) = path.and_then(Path::parent) {
            install_module_searcher(&lua, root, loaded_modules.clone()).map_err(lua_error)?;
        }
        let render = compile_entry(&lua, source, path, luax)?
            .call::<Function>(())
            .map_err(lua_error)?;
        let mut runtime = Self {
            lua,
            render,
            hooks,
            stores,
            memo,
            arena,
            host_handles,
            focus_request,
            root: None,
            handlers: HashMap::new(),
            loaded_modules,
            next_id: 1,
        };
        runtime.render(tree, false)?;
        Ok(runtime)
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
        let previous_render = std::mem::replace(&mut self.render, render);
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
                        self.render = previous_render;
                        self.hooks.lock().unwrap().restore(hook_snapshot);
                        self.stores.lock().unwrap().restore(store_snapshot);
                        self.restore_modules(module_snapshot).map_err(lua_error)?;
                        return Err(error);
                    };
                    if !reset_components.insert(mismatch.clone()) {
                        self.render = previous_render;
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
                    self.render = previous_render;
                    self.hooks.lock().unwrap().restore(hook_snapshot);
                    self.stores.lock().unwrap().restore(store_snapshot);
                    self.restore_modules(module_snapshot).map_err(lua_error)?;
                    return Err(error);
                }
            }
        }
    }

    pub(crate) fn dispatch_event(
        &mut self,
        payload: EventPayload,
        tree: &mut RetainedTree,
    ) -> Result<bool, String> {
        *self.focus_request.lock().unwrap() = None;
        let id = payload.element_id as u64;
        let Some(handler) = self
            .handlers
            .get(&(id, payload.event_type.clone()))
            .cloned()
        else {
            return Ok(false);
        };
        let event = event_table(&self.lua, &payload).map_err(lua_error)?;
        handler.call::<()>(event).map_err(lua_error)?;
        let dirty = self.hooks.lock().unwrap().dirty;
        if dirty {
            self.render(tree, false)?;
        }
        Ok(dirty)
    }

    pub(crate) fn take_focus_request(&self) -> Option<u64> {
        self.focus_request.lock().unwrap().take()
    }

    fn render(&mut self, tree: &mut RetainedTree, refreshing: bool) -> Result<(), String> {
        for pass in 0..25 {
            self.render_once(tree, refreshing && pass == 0)?;
            if !self.hooks.lock().unwrap().dirty {
                return Ok(());
            }
        }
        Err("Lua hooks scheduled too many consecutive renders".to_string())
    }

    fn render_once(&mut self, tree: &mut RetainedTree, refreshing: bool) -> Result<(), String> {
        let hook_snapshot = {
            let mut hooks = self.hooks.lock().unwrap();
            let snapshot = hooks.snapshot();
            hooks.begin_render(refreshing);
            snapshot
        };
        self.memo.lock().unwrap().seen.clear();
        self.arena.lock().unwrap().begin_render();

        let value = match self.render.call::<Value>(()) {
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
            entries.retain(|key, _| seen.contains(key));
        }

        let old = self.root.take();
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
        *self.host_handles.lock().unwrap() = handle_aliases;
        self.handlers.clear();
        collect_handlers(&root, &mut self.handlers);
        self.root = Some(root);
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
    if component.0.is_empty() {
        return "root".to_string();
    }
    let mut label = String::from("root");
    for slot in &component.0 {
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
    };
    let Some(value_type) = signature.value_type else {
        return kind.to_string();
    };
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
        lua.create_function(
            move |lua, (selector, equality): (Option<Function>, Option<Function>)| {
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
            },
        )?,
    )?;

    Ok(store)
}

fn install_api(
    lua: &Lua,
    hooks: Arc<Mutex<HookStore>>,
    stores: Arc<Mutex<StoreRegistry>>,
    memo: Arc<Mutex<MemoStore>>,
    arena: Arc<Mutex<RenderArena>>,
    host_handles: Arc<Mutex<HostHandleMap>>,
    focus_request: Arc<Mutex<Option<u64>>>,
    styles: Arc<Mutex<StyleCache>>,
) -> mlua::Result<()> {
    let api = lua.create_table()?;

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
        let id = host_handles
            .lock()
            .unwrap()
            .get(&handle)
            .copied()
            .ok_or_else(|| mlua::Error::runtime("gpuix.focus expects a mounted host handle"))?;
        *focus_request.lock().unwrap() = Some(id);
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
    let use_state = lua.create_function(move |lua, initial: Value| {
        let signature = HookSignature {
            kind: HookKind::State,
            value_type: Some(hook_value_type(&initial)),
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
    let use_reducer = lua.create_function(move |lua, (reducer, initial): (Function, Value)| {
        let signature = HookSignature {
            kind: HookKind::Reducer,
            value_type: Some(hook_value_type(&initial)),
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
    let use_ref = lua.create_function(move |lua, initial: Value| {
        let signature = HookSignature {
            kind: HookKind::Ref,
            value_type: Some(hook_value_type(&initial)),
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
    let use_memo = lua.create_function(
        move |lua, (factory, dependencies): (Function, Option<Table>)| {
            let dependencies = hook_dependencies(lua, dependencies)?;
            let signature = HookSignature {
                kind: HookKind::Memo,
                value_type: None,
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
        },
    )?;
    api.set("use_memo", use_memo)?;

    let callback_hooks = hooks.clone();
    let use_callback = lua.create_function(
        move |lua, (callback, dependencies): (Function, Option<Table>)| {
            let dependencies = hook_dependencies(lua, dependencies)?;
            let signature = HookSignature {
                kind: HookKind::Callback,
                value_type: None,
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
        },
    )?;
    api.set("use_callback", use_callback)?;

    let effect_hooks = hooks.clone();
    let use_effect = lua.create_function(
        move |lua, (callback, dependencies): (Function, Option<Table>)| {
            let dependencies = hook_dependencies(lua, dependencies)?;
            let signature = HookSignature {
                kind: HookKind::Effect,
                value_type: None,
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
        },
    )?;
    api.set("use_effect", use_effect)?;

    let memo_arena = arena.clone();
    let memo_store = memo.clone();
    let memo_hooks = hooks.clone();
    let memo_fn = lua.create_function(
        move |_, (key, dependencies, render): (Value, Value, Function)| {
            let key = memo_key(&key)?;
            let dependencies = memo_dependency(&dependencies)?;
            if let Some(cached) = memo_lookup(&memo_store, &key, &dependencies)? {
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
                let handle = if let Some(cached) = memo_lookup(&memo, &key, &dependency)? {
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
    key: &MemoKey,
    dependencies: &MemoDependency,
) -> mlua::Result<Option<MemoHit>> {
    let mut memo = memo.lock().unwrap();
    if !memo.seen.insert(key.clone()) {
        return Err(mlua::Error::runtime(format!(
            "duplicate gpuix.memo key {key:?}"
        )));
    }
    Ok(memo
        .entries
        .get(key)
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
        key,
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

    fn dispatch_by_test_id(
        runtime: &mut LuaRuntime,
        tree: &mut RetainedTree,
        test_id: &str,
    ) -> Result<bool, String> {
        let element_id = tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some(test_id))
            .unwrap()
            .id;
        runtime.dispatch_event(
            EventPayload {
                element_id: element_id as f64,
                event_type: "click".to_string(),
                ..Default::default()
            },
            tree,
        )
    }

    fn has_text(tree: &RetainedTree, content: &str) -> bool {
        tree.elements
            .values()
            .any(|element| element.content.as_deref() == Some(content))
    }
}
