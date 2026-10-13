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
//! A call fails if JS threw into it, a promise job threw, a promise was
//! rejected with no handler by the time the jobs ran out, or the app called
//! `reportError`. The rest of the call's work still runs, and every failure
//! is in the error, in order.

// The app binary uses this from Phase 4 of the design; until then only the
// tests do.
#![cfg_attr(not(test), allow(dead_code))]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use std::panic::{catch_unwind, AssertUnwindSafe};

use rquickjs::context::EvalOptions;
use rquickjs::{qjs, Context, Ctx, Exception, Function, Object, Persistent, Runtime, Value};

use crate::element_tree::EventPayload;
use crate::renderer::apply_batch_to_tree;
use crate::retained_tree::RetainedTree;

#[cfg(test)]
mod tests;

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
}

impl Default for QuickJsOptions {
    /// The wall clock, and `console` to stderr.
    fn default() -> Self {
        let start = Instant::now();
        Self {
            clock: Box::new(move || start.elapsed().as_secs_f64() * 1000.0),
            console: Box::new(|_, message| eprintln!("{message}")),
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
}

/// The prelude's hooks, saved before app code runs, so an app that writes to
/// `__gpuix` doesn't replace them.
struct Hooks {
    dispatch_event: Persistent<Function<'static>>,
    due_timers: Persistent<Function<'static>>,
    run_timer: Persistent<Function<'static>>,
    next_timer: Persistent<Function<'static>>,
    describe: Persistent<Function<'static>>,
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
        // The runtime's rejection tracker owns a handle to `shared`, so these
        // would otherwise outlive the runtime. Each call clears them, but a
        // call that unwound may not have.
        self.shared.rejections.borrow_mut().clear();
    }
}

impl QuickJsRuntime {
    /// Start a runtime and run `source` as a classic script. `file_name`
    /// names it in stack traces.
    pub(crate) fn load(
        source: &str,
        file_name: &str,
        tree: &mut RetainedTree,
        options: QuickJsOptions,
    ) -> Result<Self, String> {
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
        });
        let tracked = shared.clone();
        runtime.set_host_promise_rejection_tracker(Some(Box::new(
            move |ctx, promise, reason, handled| {
                track_rejection(&tracked, ctx, promise, reason, handled)
            },
        )));
        let hooks = context.with(|ctx| -> Result<Hooks, String> {
            let setup = (|| {
                install_host(&ctx, &shared)?;
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
        let js = Self {
            hooks,
            shared,
            context,
            runtime,
        };
        js.call(tree, |ctx, _| {
            let mut options = EvalOptions::default();
            options.strict = false;
            options.filename = Some(file_name.to_string());
            ctx.eval_with_options::<(), _>(source, options)
        })?;
        Ok(js)
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

fn install_host<'js>(ctx: &Ctx<'js>, shared: &Rc<Shared>) -> rquickjs::Result<()> {
    let host = Object::new(ctx.clone())?;

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
                    apply_batch_to_tree(&mut batches.tree.borrow_mut(), json.as_bytes())
                        .map_err(|error| Exception::throw_message(&ctx, &error))
                })
            },
        )?,
    )?;
    host.set("renderer", renderer)?;

    ctx.globals().set("__gpuix", host)
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
            Err(Exception::throw_internal(ctx, &error))
        }
    }
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
