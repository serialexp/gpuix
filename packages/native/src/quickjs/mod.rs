//! The embedded JavaScript engine: quickjs-ng through rquickjs, behind the
//! `quickjs` feature. See `docs/design/embedded-quickjs-runtime.md`.
//!
//! JS changes the retained tree the way it does under Node: GPUIX's
//! `createMutationQueue` sends each commit as one JSON batch to
//! `__gpuix.renderer.applyBatch`, which here is a native function over
//! [`apply_batch_to_tree`]. The runtime borrows the caller's tree for each
//! call that runs JS and hands it back when the call returns.
//!
//! `prelude.js` installs the timers, `console`, `performance` and
//! `structuredClone` the frameworks expect, and the hooks Rust calls. After
//! every call into JS the runtime runs promise jobs (microtasks) until none is
//! left. [`QuickJsRuntime::run_timers`] runs the timers that are due, and
//! [`QuickJsRuntime::next_timer`] says when to call it again.
//!
//! Apps are ES modules: [`QuickJsRuntime::load_module`] imports the entry
//! file, and `modules.rs` resolves its imports to app files (compiled by
//! `compile.rs`) or to the embedded framework bundles.
//!
//! A call fails if JS threw into it, a promise job threw, a promise was
//! rejected with no handler by the time the jobs ran out, or the app called
//! `reportError`. The rest of the call's work still runs, and every failure
//! is in the error, in order.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use std::panic::{catch_unwind, AssertUnwindSafe};

use rquickjs::context::EvalOptions;
use rquickjs::function::This;
use rquickjs::{
    qjs, Context, Ctx, Exception, Function, Module, Object, Persistent, Promise, Runtime, Value,
};

use crate::element_tree::EventPayload;
use crate::renderer::{apply_batch_to_tree, WindowOptions};
use crate::retained_tree::RetainedTree;

mod compile;
mod modules;
#[cfg(test)]
mod tests;

pub(crate) use compile::EXTENSIONS;

const PRELUDE: &str = include_str!("prelude.js");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConsoleLevel {
    Log,
    Info,
    Debug,
    Warn,
    Error,
}

impl ConsoleLevel {
    fn parse(level: &str) -> Self {
        match level {
            "info" => Self::Info,
            "debug" => Self::Debug,
            "warn" => Self::Warn,
            "error" => Self::Error,
            _ => Self::Log,
        }
    }
}

/// Receives each `console` call's level and formatted message.
pub(crate) type ConsoleSink = Box<dyn FnMut(ConsoleLevel, &str)>;

pub(crate) struct QuickJsOptions {
    /// Milliseconds since the runtime started. `performance.now()` and the
    /// timers read it.
    pub clock: Box<dyn Fn() -> f64>,
    /// Where `console` writes.
    pub console: ConsoleSink,
    /// What `process.env` holds.
    pub env: Vec<(String, String)>,
}

impl Default for QuickJsOptions {
    /// The wall clock, `console` to stderr, and the process's environment
    /// (minus any variable that isn't UTF-8).
    fn default() -> Self {
        let start = Instant::now();
        Self {
            clock: Box::new(move || start.elapsed().as_secs_f64() * 1000.0),
            console: Box::new(|_, message| eprintln!("{message}")),
            env: std::env::vars_os()
                .filter_map(|(name, value)| {
                    Some((name.into_string().ok()?, value.into_string().ok()?))
                })
                .collect(),
        }
    }
}

/// What the native functions JS calls share with the runtime.
struct Shared {
    /// The caller's tree while a call runs JS; an empty one in between.
    tree: RefCell<RetainedTree>,
    lent: Cell<bool>,
    /// What failed during the current call, in order.
    errors: RefCell<Vec<String>>,
    /// Promises rejected with no handler so far, with their reasons.
    rejections: RefCell<Vec<(Persistent<Value<'static>>, Persistent<Value<'static>>)>>,
    clock: Box<dyn Fn() -> f64>,
    console: RefCell<ConsoleSink>,
    /// Each compiled app module's positions, by module name, for stack
    /// traces.
    positions: RefCell<HashMap<String, compile::PositionMap>>,
    /// The options the app's renderer passed to `init`, once it has.
    window_options: RefCell<Option<WindowOptions>>,
    /// Whether a batch changed the tree since the host last asked.
    tree_changed: Cell<bool>,
    /// The entry module's name, whose `import.meta.main` is true.
    entry: Option<String>,
}

/// The prelude's hooks, saved before app code runs, so an app that writes to
/// `__gpuix` doesn't replace them.
struct Hooks {
    dispatch_event: Persistent<Function<'static>>,
    due_timers: Persistent<Function<'static>>,
    run_timer: Persistent<Function<'static>>,
    next_timer: Persistent<Function<'static>>,
    describe: Persistent<Function<'static>>,
    /// The global `reportError`.
    report_error: Persistent<Function<'static>>,
}

impl Hooks {
    fn get(ctx: &Ctx<'_>) -> rquickjs::Result<Self> {
        let host: Object = ctx.globals().get("__gpuix")?;
        let hook = |name: &str| -> rquickjs::Result<Persistent<Function<'static>>> {
            Ok(Persistent::save(ctx, host.get::<_, Function>(name)?))
        };
        Ok(Self {
            dispatch_event: hook("dispatchEvent")?,
            due_timers: hook("dueTimers")?,
            run_timer: hook("runTimer")?,
            next_timer: hook("nextTimer")?,
            describe: hook("describe")?,
            report_error: Persistent::save(ctx, ctx.globals().get::<_, Function>("reportError")?),
        })
    }
}

/// Lends the caller's tree to [`Shared`] until dropped.
struct LentTree<'a> {
    shared: &'a Shared,
    tree: &'a mut RetainedTree,
}

impl<'a> LentTree<'a> {
    fn new(shared: &'a Shared, tree: &'a mut RetainedTree) -> Self {
        std::mem::swap(tree, &mut shared.tree.borrow_mut());
        shared.lent.set(true);
        Self { shared, tree }
    }
}

impl Drop for LentTree<'_> {
    fn drop(&mut self) {
        std::mem::swap(self.tree, &mut self.shared.tree.borrow_mut());
        self.shared.lent.set(false);
    }
}

pub(crate) struct QuickJsRuntime {
    // Fields drop in order: the saved values must go before the runtime.
    hooks: Hooks,
    shared: Rc<Shared>,
    context: Context,
    runtime: Runtime,
}

impl Drop for QuickJsRuntime {
    fn drop(&mut self) {
        // The hook points at `shared`, which may go before the runtime.
        self.set_source_position_hook(std::ptr::null());
        // The runtime's rejection tracker owns a handle to `shared`, so these
        // would otherwise outlive the runtime. Each call clears them, but a
        // call that unwound may not have.
        self.shared.rejections.borrow_mut().clear();
    }
}

impl QuickJsRuntime {
    /// Start a runtime and run `source` as a classic script. `file_name`
    /// names it in stack traces.
    #[cfg(test)]
    pub(crate) fn load(
        source: &str,
        file_name: &str,
        tree: &mut RetainedTree,
        options: QuickJsOptions,
    ) -> Result<Self, String> {
        let js = Self::start(options, None)?;
        js.call(tree, |ctx, _| {
            let mut options = EvalOptions::default();
            options.strict = false;
            options.filename = Some(file_name.to_string());
            ctx.eval_with_options::<(), _>(source, options)
        })?;
        Ok(js)
    }

    /// Start a runtime and import the app module at `entry`, with the
    /// modules it imports. It fails if one can't be found or compiled, or
    /// the entry throws while it runs. An entry still waiting on top-level
    /// `await` keeps running, and a failure after that fails the call it
    /// happens in.
    pub(crate) fn load_module(
        entry: &Path,
        tree: &mut RetainedTree,
        options: QuickJsOptions,
    ) -> Result<Self, String> {
        let entry = std::path::absolute(entry)
            .map_err(|error| format!("cannot load {}: {error}", entry.display()))?;
        let entry = entry
            .to_str()
            .ok_or_else(|| format!("cannot load {}: the path isn't UTF-8", entry.display()))?;
        // Named as the loader will name it, so `import.meta.main` finds it
        // even when `app.js` loads `app.ts`. A missing entry keeps its
        // path, and the import reports it.
        let module_name = modules::resolve("", entry).unwrap_or_else(|_| entry.to_string());
        let js = Self::start(options, Some(module_name))?;
        js.call(tree, |ctx, hooks| {
            // The entry runs inside `import`, so its failure is reported
            // with the prelude's `reportError`, which the app can't replace.
            let report = hooks.report_error.clone().restore(ctx)?;
            let promise: Promise = Module::import(ctx, entry)?;
            promise
                .catch()?
                .call::<_, Value>((This(promise.clone()), report))?;
            Ok(())
        })?;
        Ok(js)
    }

    /// A runtime with the prelude run and the module loader installed, for
    /// the app whose entry module is `entry`.
    fn start(options: QuickJsOptions, entry: Option<String>) -> Result<Self, String> {
        let runtime = Runtime::new().map_err(|error| format!("cannot start QuickJS: {error}"))?;
        let context =
            Context::full(&runtime).map_err(|error| format!("cannot start QuickJS: {error}"))?;
        let shared = Rc::new(Shared {
            tree: RefCell::new(RetainedTree::new()),
            lent: Cell::new(false),
            errors: RefCell::new(Vec::new()),
            rejections: RefCell::new(Vec::new()),
            clock: options.clock,
            console: RefCell::new(options.console),
            positions: RefCell::new(HashMap::new()),
            window_options: RefCell::new(None),
            tree_changed: Cell::new(false),
            entry,
        });
        let tracked = shared.clone();
        runtime.set_host_promise_rejection_tracker(Some(Box::new(
            move |ctx, promise, reason, handled| {
                track_rejection(&tracked, ctx, promise, reason, handled)
            },
        )));
        let hooks = context.with(|ctx| -> Result<Hooks, String> {
            let setup = (|| {
                install_host(&ctx, &shared, &options.env)?;
                let mut options = EvalOptions::default();
                options.filename = Some("gpuix:prelude".to_string());
                ctx.eval_with_options::<(), _>(PRELUDE, options)?;
                Hooks::get(&ctx)
            })();
            // Nothing can describe an exception before the prelude ran.
            setup.map_err(|error| match error {
                rquickjs::Error::Exception => {
                    format!("the QuickJS prelude failed: {:?}", ctx.catch())
                }
                other => format!("the QuickJS prelude failed: {other}"),
            })
        })?;
        runtime.set_loader(
            modules::AppResolver(shared.clone()),
            modules::AppLoader(shared.clone()),
        );
        let js = Self {
            hooks,
            shared,
            context,
            runtime,
        };
        js.set_source_position_hook(Rc::as_ptr(&js.shared));
        Ok(js)
    }

    /// Maps stack trace positions with `shared`'s position maps, or stops
    /// mapping them when `shared` is null.
    fn set_source_position_hook(&self, shared: *const Shared) {
        self.context.with(|ctx| {
            // SAFETY: the context's runtime is live. The hook only reads
            // through `shared` while the runtime builds a stack trace, and
            // `drop` unsets it before `shared` can go.
            unsafe {
                let runtime = qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
                if shared.is_null() {
                    JS_SetSourcePositionFunc(runtime, None, std::ptr::null_mut());
                } else {
                    JS_SetSourcePositionFunc(
                        runtime,
                        Some(map_source_position),
                        shared.cast_mut().cast(),
                    );
                }
            }
        });
    }

    /// Deliver one GPUI event to the handler the app registered with
    /// `__gpuix.setEventHandler`, then run the microtasks it queued. Returns
    /// whether the handler took the event; false when there is none.
    pub(crate) fn dispatch_event(
        &mut self,
        payload: &EventPayload,
        tree: &mut RetainedTree,
    ) -> Result<bool, String> {
        let json = serde_json::to_string(payload)
            .map_err(|error| format!("cannot encode a {} event: {error}", payload.event_type))?;
        self.call(tree, |ctx, hooks| {
            hooks.dispatch_event.clone().restore(ctx)?.call((json,))
        })
    }

    /// The window options the app's renderer passed to `init`, once it has:
    /// `render()` from `@gpuix/react` calls it as it mounts.
    pub(crate) fn window_options(&self) -> Option<WindowOptions> {
        self.shared.window_options.borrow().clone()
    }

    /// Whether a batch changed the tree since the last call.
    pub(crate) fn take_tree_changed(&self) -> bool {
        self.shared.tree_changed.replace(false)
    }

    /// Run the timers due now, earliest first, each followed by the
    /// microtasks it queued. Timers they queue wait for the next call.
    pub(crate) fn run_timers(&mut self, tree: &mut RetainedTree) -> Result<(), String> {
        self.begin();
        let _lent = LentTree::new(&self.shared, tree);
        let due = self.invoke(|ctx, hooks| {
            hooks
                .due_timers
                .clone()
                .restore(ctx)?
                .call::<_, Vec<f64>>(())
        });
        for id in due.unwrap_or_default() {
            self.invoke(|ctx, hooks| hooks.run_timer.clone().restore(ctx)?.call::<_, ()>((id,)));
            self.drain_jobs();
        }
        self.finish(Some(()))
    }

    /// How long until a timer is due (zero if one is already), or `None`
    /// when no timer is queued.
    pub(crate) fn next_timer(&self) -> Result<Option<Duration>, String> {
        // Only prelude code runs here, so no tree is lent. (App code could
        // still run if it patched a builtin the prelude uses; `applyBatch`
        // then throws.)
        self.begin();
        let wait =
            self.invoke(|ctx, hooks| hooks.next_timer.clone().restore(ctx)?.call::<_, f64>(()));
        let wait = self.finish(wait)?;
        if wait < 0.0 {
            return Ok(None);
        }
        // The prelude caps delays, so this only fails if it has a bug.
        Duration::try_from_secs_f64(wait / 1000.0)
            .map(Some)
            .map_err(|error| format!("the next timer is due in {wait} ms: {error}"))
    }

    /// Run `run` with the tree lent, then the microtasks it queued.
    fn call<T>(
        &self,
        tree: &mut RetainedTree,
        run: impl for<'js> FnOnce(&Ctx<'js>, &Hooks) -> rquickjs::Result<T>,
    ) -> Result<T, String> {
        self.begin();
        let _lent = LentTree::new(&self.shared, tree);
        let value = self.invoke(run);
        self.drain_jobs();
        self.finish(value)
    }

    /// Forget the failures of a call that unwound before it could report
    /// them: its caller saw the panic instead.
    fn begin(&self) {
        self.shared.errors.borrow_mut().clear();
    }

    /// Run `run`, recording what it threw.
    fn invoke<T>(
        &self,
        run: impl for<'js> FnOnce(&Ctx<'js>, &Hooks) -> rquickjs::Result<T>,
    ) -> Option<T> {
        self.context.with(|ctx| match run(&ctx, &self.hooks) {
            Ok(value) => Some(value),
            Err(error) => {
                let text = describe_error(&ctx, &self.hooks, error);
                self.shared.errors.borrow_mut().push(text);
                None
            }
        })
    }

    /// Run promise jobs until none is left, then record the rejections still
    /// unhandled. Describing a rejection can run app code (a `stack` getter)
    /// that queues more, so this repeats until both are empty.
    fn drain_jobs(&self) {
        loop {
            loop {
                match self.runtime.execute_pending_job() {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(exception) => {
                        let text = exception
                            .0
                            .with(|ctx| describe_value(&ctx, &self.hooks, ctx.catch()));
                        self.shared.errors.borrow_mut().push(text);
                    }
                }
            }
            let rejections = std::mem::take(&mut *self.shared.rejections.borrow_mut());
            if rejections.is_empty() {
                return;
            }
            self.context.with(|ctx| {
                for (_, reason) in rejections {
                    let text = match reason.restore(&ctx) {
                        Ok(reason) => describe_value(&ctx, &self.hooks, reason),
                        Err(error) => error.to_string(),
                    };
                    self.shared
                        .errors
                        .borrow_mut()
                        .push(format!("Uncaught (in promise) {text}"));
                }
            });
        }
    }

    /// `value`, unless something failed during the call.
    fn finish<T>(&self, value: Option<T>) -> Result<T, String> {
        let errors = std::mem::take(&mut *self.shared.errors.borrow_mut());
        match value {
            Some(value) if errors.is_empty() => Ok(value),
            _ => Err(errors.join("\n")),
        }
    }
}

// From the vendored rquickjs-sys's `source-position` patch; see
// `vendor/README.md`.
type SourcePositionFunc = unsafe extern "C" fn(
    opaque: *mut c_void,
    file: *const c_char,
    line: *mut c_int,
    column: *mut c_int,
);

extern "C" {
    fn JS_SetSourcePositionFunc(
        runtime: *mut qjs::JSRuntime,
        func: Option<SourcePositionFunc>,
        opaque: *mut c_void,
    );
}

/// Maps a position in a compiled app module back to its source, for every
/// stack trace QuickJS builds, `Error.prepareStackTrace`'s call sites
/// included. It runs in C with the stack possibly nearly full, so it only
/// reads the position maps, and leaves a position it can't map unchanged.
unsafe extern "C" fn map_source_position(
    opaque: *mut c_void,
    file: *const c_char,
    line: *mut c_int,
    column: *mut c_int,
) {
    // SAFETY: `opaque` is the runtime's live `Shared` (see
    // `set_source_position_hook`), and QuickJS passes a valid file name and
    // position.
    let (shared, file, line, column) = unsafe {
        (
            &*opaque.cast::<Shared>(),
            CStr::from_ptr(file),
            &mut *line,
            &mut *column,
        )
    };
    let (Ok(file), Ok(positions)) = (file.to_str(), shared.positions.try_borrow()) else {
        return;
    };
    let (Ok(compiled_line), Ok(compiled_column)) = (u32::try_from(*line), u32::try_from(*column))
    else {
        return;
    };
    let Some((source_line, source_column)) = positions
        .get(file)
        .and_then(|map| map.source_position(compiled_line, compiled_column))
    else {
        return;
    };
    if let (Ok(source_line), Ok(source_column)) =
        (c_int::try_from(source_line), c_int::try_from(source_column))
    {
        *line = source_line;
        *column = source_column;
    }
}

fn install_host<'js>(
    ctx: &Ctx<'js>,
    shared: &Rc<Shared>,
    env: &[(String, String)],
) -> rquickjs::Result<()> {
    let host = Object::new(ctx.clone())?;
    host.set("process", process_info(ctx, shared.entry.as_deref(), env)?)?;

    let clock = shared.clone();
    host.set(
        "now",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            guard(&clock, &ctx, "now", || Ok((clock.clock)()))
        })?,
    )?;

    let console = shared.clone();
    host.set(
        "print",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, level: String, message: String| {
                guard(&console, &ctx, "console", || {
                    (console.console.borrow_mut())(ConsoleLevel::parse(&level), &message);
                    Ok(())
                })
            },
        )?,
    )?;

    let errors = shared.clone();
    host.set(
        "reportError",
        Function::new(ctx.clone(), move |message: String| {
            errors.errors.borrow_mut().push(message);
        })?,
    )?;

    host.set("proxyTarget", Function::new(ctx.clone(), proxy_target)?)?;

    let window = shared.clone();
    host.set(
        "init",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, json: String| {
            guard(&window, &ctx, "init", || {
                if window.window_options.borrow().is_some() {
                    return Err(Exception::throw_message(
                        &ctx,
                        "Renderer is already initialized",
                    ));
                }
                let options = serde_json::from_str::<WindowOptions>(&json).map_err(|error| {
                    throw_whole(
                        &ctx,
                        ErrorKind::Type,
                        &format!("invalid window options: {error}"),
                    )
                })?;
                *window.window_options.borrow_mut() = Some(options);
                Ok(())
            })
        })?,
    )?;

    let renderer = Object::new(ctx.clone())?;
    let batches = shared.clone();
    renderer.set(
        "applyBatch",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, json: String| -> rquickjs::Result<Vec<f64>> {
                guard(&batches, &ctx, "applyBatch", || {
                    if !batches.lent.get() {
                        return Err(Exception::throw_message(
                            &ctx,
                            "applyBatch can only run while GPUIX is calling into the app",
                        ));
                    }
                    let ids = apply_batch_to_tree(&mut batches.tree.borrow_mut(), json.as_bytes())
                        .map_err(|error| Exception::throw_message(&ctx, &error))?;
                    batches.tree_changed.set(true);
                    Ok(ids)
                })
            },
        )?,
    )?;
    host.set("renderer", renderer)?;

    ctx.globals().set("__gpuix", host)
}

/// What the prelude's `process` reports: the environment, the command line
/// as Node gives it (the program, then the entry file) and the platform, by
/// Node's names.
fn process_info<'js>(
    ctx: &Ctx<'js>,
    entry: Option<&str>,
    env: &[(String, String)],
) -> rquickjs::Result<Object<'js>> {
    let info = Object::new(ctx.clone())?;
    let variables = Object::new(ctx.clone())?;
    for (name, value) in env {
        variables.set(name.as_str(), value.as_str())?;
    }
    info.set("env", variables)?;
    let program = std::env::current_exe()
        .ok()
        .and_then(|path| path.into_os_string().into_string().ok())
        .unwrap_or_else(|| "gpx".to_string());
    let argv: Vec<String> = std::iter::once(program)
        .chain(entry.map(str::to_string))
        .collect();
    info.set("argv", argv)?;
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        os => os,
    };
    info.set("platform", platform)?;
    Ok(info)
}

/// Run a native function's body, turning a panic into a failure of the
/// call. rquickjs would turn it into a JS exception and resume the panic at
/// the next exception it sees, so an app that caught the exception would
/// leave the panic parked until some unrelated later call.
fn guard<'js, T>(
    shared: &Shared,
    ctx: &Ctx<'js>,
    name: &str,
    run: impl FnOnce() -> rquickjs::Result<T>,
) -> rquickjs::Result<T> {
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(result) => result,
        Err(panic) => {
            let message = panic
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("no message");
            let error = format!("GPUIX panicked in {name}: {message}");
            shared.errors.borrow_mut().push(error.clone());
            Err(throw_whole(ctx, ErrorKind::Internal, &error))
        }
    }
}

pub(crate) enum ErrorKind {
    Syntax,
    Type,
    Internal,
}

/// Throws a `SyntaxError`, `TypeError` or `InternalError` with all of
/// `message`. rquickjs's `Exception::throw_syntax` and the like format it
/// into a 256-byte buffer, which an absolute path and one note already fill.
pub(crate) fn throw_whole(ctx: &Ctx<'_>, kind: ErrorKind, message: &str) -> rquickjs::Error {
    let raw = ctx.as_raw().as_ptr();
    // SAFETY: `raw` is this live context. The error comes back as a new
    // reference, which `from_raw` takes; on failure it is the exception
    // marker, with the exception already pending.
    let error = unsafe {
        let value = match kind {
            ErrorKind::Syntax => qjs::JS_NewSyntaxError(raw, c"%s".as_ptr(), c"".as_ptr()),
            ErrorKind::Type => qjs::JS_NewTypeError(raw, c"%s".as_ptr(), c"".as_ptr()),
            ErrorKind::Internal => qjs::JS_NewInternalError(raw, c"%s".as_ptr(), c"".as_ptr()),
        };
        Value::from_raw(ctx.clone(), value)
    };
    if error.is_exception() {
        return rquickjs::Error::Exception;
    }
    if let Some(object) = error.as_object() {
        if let Err(failure) = object.set("message", message) {
            return failure;
        }
    }
    ctx.throw(error)
}

/// A proxy's target (`null` once revoked), or `undefined` for anything else.
/// The console prints the target so it runs no traps.
fn proxy_target<'js>(ctx: Ctx<'js>, value: Value<'js>) -> Value<'js> {
    // SAFETY: `value` is a live value of this context, and the target that
    // `JS_GetProxyTarget` returns is a new reference that `from_raw` takes.
    unsafe {
        if !qjs::JS_IsProxy(value.as_raw()) {
            return Value::new_undefined(ctx);
        }
        let target = qjs::JS_GetProxyTarget(ctx.as_raw().as_ptr(), value.as_raw());
        if qjs::JS_IsException(target) {
            // Revoked.
            ctx.catch();
            return Value::new_null(ctx);
        }
        Value::from_raw(ctx, target)
    }
}

fn track_rejection<'js>(
    shared: &Shared,
    ctx: Ctx<'js>,
    promise: Value<'js>,
    reason: Value<'js>,
    handled: bool,
) {
    let mut rejections = shared.rejections.borrow_mut();
    if handled {
        rejections.retain(|(rejected, _)| {
            rejected
                .clone()
                .restore(&ctx)
                .map_or(true, |rejected| rejected != promise)
        });
    } else {
        rejections.push((
            Persistent::save(&ctx, promise),
            Persistent::save(&ctx, reason),
        ));
    }
}

fn describe_error(ctx: &Ctx<'_>, hooks: &Hooks, error: rquickjs::Error) -> String {
    match error {
        rquickjs::Error::Exception => describe_value(ctx, hooks, ctx.catch()),
        other => other.to_string(),
    }
}

/// An exception as the console would print it: an error's name, message
/// and stack, or any other value.
fn describe_value<'js>(ctx: &Ctx<'js>, hooks: &Hooks, value: Value<'js>) -> String {
    let described = (|| {
        hooks
            .describe
            .clone()
            .restore(ctx)?
            .call::<_, String>((value,))
    })();
    described.unwrap_or_else(|_| {
        ctx.catch();
        "an exception that could not be described".to_string()
    })
}
