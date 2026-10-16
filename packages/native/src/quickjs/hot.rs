//! Hot updates for `gpx --watch`, with React Fast Refresh.
//!
//! In [`Mode::Development`] the runtime loads the frameworks' development
//! builds and `react-refresh/runtime`, and installs its hook before any app
//! module runs, so `@gpuix/react`'s renderer registers with it
//! (`injectIntoDevTools`). React files compile with React Refresh's
//! registrations (`compile.rs`), which call the `import.meta.$RefreshReg$`
//! and `import.meta.$RefreshSig$` the loader gives each of them: components
//! register under their file's path and name, so a new version of a file
//! registers into the same families.
//!
//! [`QuickJsRuntime::hot_update`] loads each changed file again as a new
//! module, which `modules.rs` names with its version. QuickJS can't run a
//! module again in place, and the modules that import the old one keep it.
//! That's enough when the module exports only components, and the same
//! names as before: React swaps each family to its new version and keeps
//! the components' state. A module that doesn't (a file not compiled for
//! React Refresh, as a Solid file, among them) is loaded again in the
//! modules that import it, up the import graph, until each path reaches
//! one that does. One that reaches the entry file can't be updated in
//! place, and the app starts again.
//!
//! As Vite does, the update works out every module it loads again before it
//! runs any, gives them all new versions, and then imports only the modules
//! it stops at. QuickJS then loads and runs each new version once, the
//! modules each one imports first, so none of them runs with an old version
//! of another. A boundary whose exports changed brings its importers in, and
//! the update starts over with the bigger set.
//!
//! Like other hot reloaders, the old module's side effects stay: a timer it
//! started keeps running, and the old module stays in memory.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use rquickjs::function::This;
use rquickjs::{Ctx, Function, Module, Object, Persistent, Promise, Value};

use super::modules::{is_embedded, module_file, text_module, versioned};
use super::{Mode, QuickJsRuntime, Shared};

/// What a hot update did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HotUpdate {
    /// The files ran again, in this order, and React swapped their
    /// components in place.
    Refreshed(Vec<String>),
    /// No file the app runs changed.
    Unchanged,
    /// The change can't be made in place, for the reason given; the app has
    /// to start again.
    Restart(String),
}

/// The app modules a runtime loaded, the app modules each one imports, and
/// which version of each file imports get. Modules are named by their
/// unversioned module name: a file's path, or its text module's name.
#[derive(Clone, Default)]
pub(crate) struct Graph {
    /// Each module the app loaded: whether it registers with React
    /// Refresh, and the version that last loaded.
    loaded: HashMap<String, (bool, u32)>,
    /// The modules each module's newest version imports.
    imports: HashMap<String, BTreeSet<String>>,
    /// The modules that import each module.
    importers: HashMap<String, BTreeSet<String>>,
    /// Each module's newest version, once a hot update loaded it again.
    versions: HashMap<String, u32>,
    /// The last version handed out. Versions aren't reused, as QuickJS keeps
    /// every module it declared, a failed one too.
    last_version: u32,
    /// The modules loaded since the last hot update started, in order.
    loads: Vec<String>,
    /// Changed modules a failed hot update didn't bring up to date, which
    /// the next one does.
    pending: BTreeSet<String>,
}

impl Graph {
    /// The version of `module` that imports get.
    pub(crate) fn version(&self, module: &str) -> u32 {
        self.versions.get(module).copied().unwrap_or(0)
    }

    /// Records that the app module named `importer`, a version of
    /// `importer_module`, imports the app module `module`. Only the newest
    /// version's imports count: an old version still running, as a timer's
    /// `import()`, isn't what a change has to reach.
    pub(crate) fn imported(&mut self, importer: &str, importer_module: &str, module: &str) {
        if importer.is_empty()
            || is_embedded(importer)
            || importer != versioned(importer_module, self.version(importer_module))
        {
            return;
        }
        let importer = importer_module;
        self.imports
            .entry(importer.to_string())
            .or_default()
            .insert(module.to_string());
        self.importers
            .entry(module.to_string())
            .or_default()
            .insert(importer.to_string());
    }

    /// Forgets what `module` imports, before a new version of it records
    /// its own imports.
    pub(crate) fn forget_imports(&mut self, module: &str) {
        for imported in self.imports.remove(module).unwrap_or_default() {
            if let Some(importers) = self.importers.get_mut(&imported) {
                importers.remove(module);
            }
        }
    }

    /// Records that `module`'s newest version loaded, and whether it
    /// registers with React Refresh.
    pub(crate) fn loaded(&mut self, module: &str, refresh: bool) {
        let version = self.version(module);
        self.loaded.insert(module.to_string(), (refresh, version));
        self.loads.push(module.to_string());
    }

    /// Whether `module`'s newest version loaded and registers with React
    /// Refresh, so the app runs it and it may stop an update. A version a
    /// hot update gave it that nothing imported since doesn't run.
    fn runs_refreshable(&self, module: &str) -> bool {
        self.loaded.get(module) == Some(&(true, self.version(module)))
    }

    /// Gives `module` a new version, which imports get from now.
    fn bump(&mut self, module: &str) {
        self.last_version += 1;
        self.versions.insert(module.to_string(), self.last_version);
    }

    fn importers(&self, module: &str) -> Vec<String> {
        self.importers
            .get(module)
            .map(|importers| importers.iter().cloned().collect())
            .unwrap_or_default()
    }
}

/// The modules a hot update loads again.
#[derive(Default)]
struct Plan {
    /// Each of them, the changed ones first, then their importers.
    modules: Vec<String>,
    planned: HashSet<String>,
    /// The ones it stops at, as each exports only components, with the
    /// names it exported before.
    boundaries: Vec<(String, Vec<String>)>,
}

impl Plan {
    /// Adds `module`, and says whether it's new to the plan.
    fn add(&mut self, module: &str) -> bool {
        let new = self.planned.insert(module.to_string());
        if new {
            self.modules.push(module.to_string());
        }
        new
    }
}

/// What a change to one file means for the app.
enum Change {
    /// It changed these app modules: the file's code, its text, or both.
    Modules(Vec<String>),
    /// The app loaded it, and it's gone.
    Removed(String),
}

/// `react-refresh/runtime`'s functions, saved once it loads.
pub(crate) struct Refresh {
    /// Takes a file's path, and returns its `$RefreshReg$`.
    register_for: Persistent<Function<'static>>,
    create_signature: Persistent<Function<'static>>,
    perform: Persistent<Function<'static>>,
    is_component: Persistent<Function<'static>>,
}

/// Gives the module at `path` the `import.meta.$RefreshReg$` and
/// `$RefreshSig$` its compiled code calls.
pub(crate) fn provide_refresh<'js>(
    ctx: &Ctx<'js>,
    shared: &Shared,
    meta: &Object<'js>,
    path: &str,
) -> rquickjs::Result<()> {
    let refresh = shared.refresh.borrow();
    let Some(refresh) = refresh.as_ref() else {
        return Ok(());
    };
    let register_for = refresh.register_for.clone().restore(ctx)?;
    meta.set("$RefreshReg$", register_for.call::<_, Value>((path,))?)?;
    meta.set(
        "$RefreshSig$",
        refresh.create_signature.clone().restore(ctx)?,
    )?;
    Ok(())
}

impl QuickJsRuntime {
    /// Loads `react-refresh/runtime` and installs its hook, before the app
    /// loads React.
    pub(super) fn install_refresh(&self) -> Result<(), String> {
        debug_assert_eq!(self.shared.mode, Mode::Development);
        let runtime = self.import_now("react-refresh/runtime")?;
        let refresh = self.call(|ctx, _| {
            let runtime = runtime.restore(ctx)?;
            let inject: Function = runtime.get("injectIntoGlobalHook")?;
            inject.call::<_, ()>((ctx.globals(),))?;
            let factory: Function =
                ctx.eval("(register) => (path) => (type, id) => register(type, path + ' ' + id)")?;
            let register_for: Function =
                factory.call((runtime.get::<_, Function>("register")?,))?;
            let save = |name: &str| -> rquickjs::Result<Persistent<Function<'static>>> {
                Ok(Persistent::save(ctx, runtime.get::<_, Function>(name)?))
            };
            Ok(Refresh {
                register_for: Persistent::save(ctx, register_for),
                create_signature: save("createSignatureFunctionForTransform")?,
                perform: save("performReactRefresh")?,
                is_component: save("isLikelyComponentType")?,
            })
        })?;
        *self.shared.refresh.borrow_mut() = Some(refresh);
        Ok(())
    }

    /// Brings the app up to date with the files at `changed`, in place if
    /// it can (see the module docs). A file that fails to compile or run
    /// fails the update, and the app runs on as it was; the next update
    /// brings in this one's changes too.
    pub(crate) fn hot_update(&self, changed: &[PathBuf]) -> Result<HotUpdate, String> {
        if self.shared.refresh.borrow().is_none() {
            return Ok(HotUpdate::Restart(
                "this app runs the frameworks' production builds".to_string(),
            ));
        }
        let mut modules: Vec<String> = std::mem::take(&mut self.shared.graph.borrow_mut().pending)
            .into_iter()
            .collect();
        for path in changed {
            match self.change(path) {
                Change::Modules(changed) => {
                    for module in changed {
                        if !modules.contains(&module) {
                            modules.push(module);
                        }
                    }
                }
                Change::Removed(file) => {
                    return Ok(HotUpdate::Restart(format!("{file} was removed")))
                }
            }
        }
        if modules.is_empty() {
            return Ok(HotUpdate::Unchanged);
        }

        let saved = {
            let mut graph = self.shared.graph.borrow_mut();
            graph.loads.clear();
            graph.clone()
        };
        let update = self.update(&modules);
        if update.is_err() {
            let mut graph = self.shared.graph.borrow_mut();
            let failed = std::mem::replace(&mut *graph, saved);
            graph.last_version = failed.last_version;
            graph.pending = modules.into_iter().collect();
            // QuickJS keeps the modules the update loaded under their names,
            // failed ones too. A module it loaded first keeps a new version,
            // so the next import loads it again, and the app watches it.
            for module in &failed.loads {
                if !graph.loaded.contains_key(module) {
                    let refresh = failed
                        .loaded
                        .get(module)
                        .is_some_and(|(refresh, _)| *refresh);
                    graph.bump(module);
                    graph.loaded.insert(module.clone(), (refresh, 0));
                }
            }
        }
        update
    }

    /// The app modules the file at `path` is, if the app loaded it.
    fn change(&self, path: &Path) -> Change {
        let Ok(absolute) = std::path::absolute(path) else {
            return Change::Modules(Vec::new());
        };
        let exists = absolute.exists();
        // Modules are named by canonical path. A removed file has none, but
        // its directory may still, through a link (macOS's /var, say).
        let canonical = std::fs::canonicalize(&absolute).ok().or_else(|| {
            let directory = std::fs::canonicalize(absolute.parent()?).ok()?;
            Some(directory.join(absolute.file_name()?))
        });
        let Some(file) = canonical.as_deref().and_then(Path::to_str) else {
            return Change::Modules(Vec::new());
        };
        let graph = self.shared.graph.borrow();
        let modules: Vec<String> = [file.to_string(), text_module(file)]
            .into_iter()
            .filter(|module| graph.loaded.contains_key(module))
            .collect();
        if !exists && !modules.is_empty() {
            return Change::Removed(file.to_string());
        }
        Change::Modules(modules)
    }

    /// Loads `changed` again, with the modules that have to load again with
    /// them, and has React swap their components.
    fn update(&self, changed: &[String]) -> Result<HotUpdate, String> {
        let mut plan = Plan::default();
        if let Some(reason) = self.spread(&mut plan, changed.to_vec())? {
            return Ok(HotUpdate::Restart(reason));
        }
        // Each pass gives every planned module a new version and imports
        // the boundaries, which load the new versions of what they import.
        // A boundary whose exports changed brings its importers into the
        // plan, and a boundary imported before them may have loaded them
        // as they were, so the next pass starts over. The plan only grows,
        // so the passes end.
        'pass: loop {
            {
                let mut graph = self.shared.graph.borrow_mut();
                for module in &plan.modules {
                    graph.bump(module);
                }
            }
            for index in 0..plan.boundaries.len() {
                let (boundary, names) = plan.boundaries[index].clone();
                let namespace = self.import_now(&boundary)?;
                if self.component_exports(namespace)? == Some(names) {
                    continue;
                }
                // It no longer stops the update: what imports it binds to
                // what it exports.
                let planned = plan.modules.len();
                let importers = self.shared.graph.borrow().importers(&boundary);
                if let Some(reason) = self.spread(&mut plan, importers)? {
                    return Ok(HotUpdate::Restart(reason));
                }
                if plan.modules.len() > planned {
                    continue 'pass;
                }
            }
            break;
        }

        // The files that ran, in the plan's order, then any the new
        // versions imported for the first time.
        let mut ran: Vec<String> = Vec::new();
        {
            let graph = self.shared.graph.borrow();
            let loads: HashSet<&String> = graph.loads.iter().collect();
            let planned = plan.modules.iter().filter(|module| loads.contains(module));
            let first_time = graph
                .loads
                .iter()
                .filter(|module| !plan.planned.contains(*module));
            for module in planned.chain(first_time) {
                let file = module_file(module);
                if !ran.iter().any(|ran| ran == file) {
                    ran.push(file.to_string());
                }
            }
        }
        if ran.is_empty() {
            return Ok(HotUpdate::Unchanged);
        }
        self.call(|ctx, _| {
            let refresh = self.shared.refresh.borrow();
            let refresh = refresh.as_ref().expect("checked above");
            refresh.perform.clone().restore(ctx)?.call::<_, Value>(())?;
            Ok(())
        })?;
        Ok(HotUpdate::Refreshed(ran))
    }

    /// Adds `start` to `plan`, and the modules that have to load again with
    /// them, up the import graph to modules that export only components, or
    /// else the reason the app has to start again.
    fn spread(&self, plan: &mut Plan, start: Vec<String>) -> Result<Option<String>, String> {
        let mut queue: VecDeque<String> = start
            .into_iter()
            .filter(|module| plan.add(module))
            .collect();
        while let Some(module) = queue.pop_front() {
            if self.shared.entry.as_deref() == Some(module.as_str()) {
                return Ok(Some(format!("{module}, the entry file, would run again")));
            }
            let refreshable = self.shared.graph.borrow().runs_refreshable(&module);
            if refreshable {
                // The version running now, which loaded already, so this
                // doesn't run it again.
                let namespace = self.import_now(&module)?;
                if let Some(names) = self.component_exports(namespace)? {
                    plan.boundaries.push((module, names));
                    continue;
                }
            }
            // One nothing imports any more, but the entry, is code the app
            // no longer reaches.
            let importers = self.shared.graph.borrow().importers(&module);
            for importer in importers {
                if plan.add(&importer) {
                    queue.push_back(importer);
                }
            }
        }
        Ok(None)
    }

    /// The names `namespace` exports, if it exports something, and only
    /// what React Refresh takes for components.
    fn component_exports(
        &self,
        namespace: Persistent<Object<'static>>,
    ) -> Result<Option<Vec<String>>, String> {
        self.call(|ctx, _| {
            let namespace = namespace.restore(ctx)?;
            let refresh = self.shared.refresh.borrow();
            let is_component = refresh
                .as_ref()
                .expect("hot updates need React Refresh")
                .is_component
                .clone()
                .restore(ctx)?;
            let mut names = Vec::new();
            for name in namespace.keys::<String>() {
                let name = name?;
                let value: Value = namespace.get(&name)?;
                if !is_component.call::<_, bool>((value,))? {
                    return Ok(None);
                }
                names.push(name);
            }
            names.sort();
            Ok((!names.is_empty()).then_some(names))
        })
    }

    /// Imports `name` and runs it, with the modules it imports, and returns
    /// its namespace. A module still waiting on top-level `await` fails.
    fn import_now(&self, name: &str) -> Result<Persistent<Object<'static>>, String> {
        let promise = self.call(|ctx, _| {
            let promise = Module::import(ctx, name)?;
            // Handled, so a failure is reported once, below, and not also
            // as an unhandled rejection.
            let ignore = Function::new(ctx.clone(), || {})?;
            promise
                .catch()?
                .call::<_, Value>((This(promise.clone()), ignore))?;
            Ok(Persistent::save(ctx, promise))
        })?;
        self.call(|ctx, _| {
            let promise: Promise = promise.restore(ctx)?;
            match promise.result::<Object>() {
                Some(namespace) => Ok(Persistent::save(ctx, namespace?)),
                None => Err(rquickjs::Exception::throw_message(
                    ctx,
                    &format!("{name} is still waiting on top-level await"),
                )),
            }
        })
    }
}
