use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use super::compile::compile;
use super::{ConsoleLevel, QuickJsOptions, QuickJsRuntime};
use crate::element_tree::{EventModifiers, EventPayload};
use crate::retained_tree::RetainedTree;

/// Sends `ops` as one batch, the way `createMutationQueue` does.
const BATCH: &str = "const batch = (...ops) => __gpuix.renderer.applyBatch(JSON.stringify(ops));\n";

type Output = Rc<RefCell<Vec<(ConsoleLevel, String)>>>;

struct App {
    js: QuickJsRuntime,
    tree: RetainedTree,
    /// What the runtime's clock reads, in milliseconds.
    time: Rc<Cell<f64>>,
    output: Output,
}

impl App {
    fn try_load(source: &str) -> Result<Self, String> {
        Self::start(|tree, options| {
            QuickJsRuntime::load(&format!("{BATCH}{source}"), "app.js", tree, options)
        })
    }

    fn try_load_module(entry: &Path) -> Result<Self, String> {
        Self::start(|tree, options| QuickJsRuntime::load_module(entry, tree, options))
    }

    /// A runtime from `load`, on a fake clock, with its console captured.
    fn start(
        load: impl FnOnce(&mut RetainedTree, QuickJsOptions) -> Result<QuickJsRuntime, String>,
    ) -> Result<Self, String> {
        let time = Rc::new(Cell::new(0.0));
        let output = Output::default();
        let clock = time.clone();
        let console = output.clone();
        let mut tree = RetainedTree::new();
        let js = load(
            &mut tree,
            QuickJsOptions {
                clock: Box::new(move || clock.get()),
                console: Box::new(move |level, message| {
                    console.borrow_mut().push((level, message.to_string()));
                }),
            },
        )?;
        Ok(Self {
            js,
            tree,
            time,
            output,
        })
    }

    fn load(source: &str) -> Self {
        Self::try_load(source).unwrap()
    }

    fn dispatch(&mut self, payload: EventPayload) -> Result<bool, String> {
        self.js.dispatch_event(&payload, &mut self.tree)
    }

    fn click(&mut self, element_id: u64) -> Result<bool, String> {
        self.dispatch(event(element_id, "click"))
    }

    /// Move the clock to `time` and run the timers due then.
    fn advance_to(&mut self, time: f64) -> Result<(), String> {
        self.time.set(time);
        self.js.run_timers(&mut self.tree)
    }

    fn text(&self, id: u64) -> Option<&str> {
        self.tree.elements.get(&id)?.content.as_deref()
    }

    fn children(&self, id: u64) -> Vec<u64> {
        self.tree.elements[&id].children.clone()
    }

    fn printed(&self) -> Vec<(ConsoleLevel, String)> {
        self.output.borrow().clone()
    }

    fn log(&self) -> Vec<String> {
        self.printed()
            .into_iter()
            .map(|(_, message)| message)
            .collect()
    }
}

fn event(element_id: u64, event_type: &str) -> EventPayload {
    EventPayload {
        element_id: element_id as f64,
        event_type: event_type.to_string(),
        ..EventPayload::default()
    }
}

#[test]
fn batches_reach_the_retained_tree() {
    let app = App::load(
        r#"
        batch(
          ["createElement", 1, "div"],
          ["createElement", 2, "text"],
          ["createElement", 3, "text"],
          ["setText", 2, "first"],
          ["setText", 3, "second"],
          ["appendChild", 1, 3],
          ["insertBefore", 1, 2, 3],
          ["setStyle", 1, { padding: 4 }],
          ["setEventListener", 2, "click", true],
          ["setCustomProp", 1, "testId", "list"],
          ["setRoot", 1],
        );
        "#,
    );
    assert_eq!(app.tree.root_id, Some(1));
    assert_eq!(app.children(1), [2, 3]);
    assert_eq!(app.text(2), Some("first"));
    assert_eq!(app.text(3), Some("second"));
    assert!(app.tree.elements[&1].style.is_some());
    assert!(app.tree.elements[&2].events().contains("click"));
    assert_eq!(app.tree.elements[&1].test_id.as_deref(), Some("list"));
}

#[test]
fn apply_batch_returns_destroyed_ids_and_throws_on_a_bad_batch() {
    let app = App::load(
        r#"
        batch(
          ["createElement", 1, "div"],
          ["createElement", 2, "div"],
          ["createElement", 3, "text"],
          ["appendChild", 1, 2],
          ["appendChild", 2, 3],
          ["setRoot", 1],
        );
        const destroyed = batch(["destroyElement", 2]);
        console.log(JSON.stringify(destroyed.sort()));
        try {
          batch(["setText", 1, "kept out"], ["noSuchOp", 1]);
        } catch (error) {
          console.log(error.message);
        }
        "#,
    );
    let log = app.log();
    assert_eq!(log[0], "[2,3]");
    assert!(log[1].contains("noSuchOp"), "{log:?}");
    // The batch is atomic: the valid op before the bad one didn't apply.
    assert_eq!(app.text(1), None);
    assert_eq!(app.children(1), Vec::<u64>::new());
}

#[test]
fn events_reach_the_registered_handler_and_microtasks_run_before_returning() {
    let mut app = App::load(
        r#"
        batch(["createElement", 1, "text"], ["setText", 1, "0"], ["setRoot", 1]);
        let count = 0;
        __gpuix.setEventHandler((event) => {
          if (event.elementId !== 1 || event.eventType !== "click") return false;
          count += 1;
          // Commits wait for a microtask, as Solid's and React's do.
          queueMicrotask(() => Promise.resolve().then(() => batch(["setText", 1, String(count)])));
          return true;
        });
        "#,
    );
    assert_eq!(app.click(1), Ok(true));
    assert_eq!(app.text(1), Some("1"));
    assert_eq!(app.click(2), Ok(false));
    assert_eq!(app.click(1), Ok(true));
    assert_eq!(app.text(1), Some("2"));
}

#[test]
fn an_event_without_a_handler_is_not_taken() {
    let mut app = App::load("");
    assert_eq!(app.click(1), Ok(false));
}

#[test]
fn events_arrive_as_the_browser_build_sends_them() {
    let mut app = App::load(
        r#"
        __gpuix.setEventHandler((event) => {
          console.log(JSON.stringify(event));
          return true;
        });
        "#,
    );
    app.dispatch(EventPayload {
        key: Some("tab".to_string()),
        keystroke_id: Some(7.0),
        modifiers: Some(EventModifiers {
            shift: true,
            ..EventModifiers::default()
        }),
        ..event(4, "keyDown")
    })
    .unwrap();
    let event: serde_json::Value = serde_json::from_str(&app.log()[0]).unwrap();
    assert_eq!(event["elementId"], 4.0);
    assert_eq!(event["eventType"], "keyDown");
    assert_eq!(event["key"], "tab");
    assert_eq!(event["keystrokeId"], 7.0);
    assert_eq!(event["modifiers"]["shift"], true);
    assert_eq!(event["modifiers"]["cmd"], false);
    // As in the browser build, an absent field is null.
    assert!(event["x"].is_null());
}

#[test]
fn timers_run_when_due_in_order() {
    let mut app = App::load(
        r#"
        setTimeout(() => console.log("late"), 100);
        setTimeout((a, b) => console.log("early", a, b), 10, "x", 1);
        setTimeout(() => console.log("also late"), 100);
        const cleared = setTimeout(() => console.log("cleared"), 50);
        clearTimeout(cleared);
        "#,
    );
    assert_eq!(app.js.next_timer(), Ok(Some(Duration::from_millis(10))));
    app.advance_to(5.0).unwrap();
    assert!(app.log().is_empty());
    app.advance_to(100.0).unwrap();
    assert_eq!(app.log(), ["early x 1", "late", "also late"]);
    assert_eq!(app.js.next_timer(), Ok(None));
}

#[test]
fn intervals_repeat_until_cleared() {
    let mut app = App::load(
        r#"
        let ticks = 0;
        const interval = setInterval(() => {
          ticks += 1;
          console.log(`tick ${ticks}`);
          if (ticks === 3) clearInterval(interval);
        }, 20);
        "#,
    );
    for time in [20.0, 40.0, 60.0, 80.0] {
        app.advance_to(time).unwrap();
    }
    assert_eq!(app.log(), ["tick 1", "tick 2", "tick 3"]);
    assert_eq!(app.js.next_timer(), Ok(None));
}

#[test]
fn a_timer_queued_by_a_timer_waits_for_the_next_round() {
    let mut app = App::load(
        r#"
        const again = () => {
          console.log("ran");
          setImmediate(again);
        };
        setTimeout(again, 0);
        "#,
    );
    app.advance_to(0.0).unwrap();
    assert_eq!(app.log(), ["ran"]);
    assert_eq!(app.js.next_timer(), Ok(Some(Duration::ZERO)));
    app.advance_to(0.0).unwrap();
    assert_eq!(app.log(), ["ran", "ran"]);
}

#[test]
fn microtasks_run_between_timers() {
    let mut app = App::load(
        r#"
        setTimeout(() => {
          console.log("first");
          queueMicrotask(() => console.log("first's microtask"));
        });
        setTimeout(() => console.log("second"));
        "#,
    );
    app.advance_to(0.0).unwrap();
    assert_eq!(app.log(), ["first", "first's microtask", "second"]);
}

#[test]
fn a_timer_cleared_by_an_earlier_timer_of_the_same_round_does_not_run() {
    let mut app = App::load(
        r#"
        let second;
        setTimeout(() => clearTimeout(second));
        second = setTimeout(() => console.log("second"));
        "#,
    );
    app.advance_to(0.0).unwrap();
    assert!(app.log().is_empty());
}

#[test]
fn timers_change_the_tree() {
    let mut app = App::load(
        r#"
        batch(["createElement", 1, "text"], ["setRoot", 1]);
        setTimeout(() => batch(["setText", 1, "later"]), 30);
        "#,
    );
    app.advance_to(30.0).unwrap();
    assert_eq!(app.text(1), Some("later"));
}

#[test]
fn performance_now_reads_the_runtime_clock() {
    let mut app = App::load(
        r#"
        __gpuix.setEventHandler(() => {
          console.log(String(performance.now()));
          return true;
        });
        "#,
    );
    app.time.set(1234.5);
    app.click(1).unwrap();
    assert_eq!(app.log(), ["1234.5"]);
}

#[test]
fn a_throwing_handler_fails_the_event_with_its_stack() {
    let mut app = App::load(
        r#"
        batch(["createElement", 1, "text"], ["setRoot", 1]);
        function explode() {
          batch(["setText", 1, "before the throw"]);
          null.boom;
        }
        __gpuix.setEventHandler(() => explode());
        "#,
    );
    let error = app.click(1).unwrap_err();
    assert!(error.starts_with("TypeError: "), "{error}");
    assert!(error.contains("explode"), "{error}");
    assert!(error.contains("app.js"), "{error}");
    // What ran before the throw stays applied.
    assert_eq!(app.text(1), Some("before the throw"));
}

#[test]
fn a_thrown_value_that_is_not_an_error_is_described() {
    let mut app = App::load(r#"__gpuix.setEventHandler(() => { throw { code: 7 }; });"#);
    assert_eq!(app.click(1), Err("{code: 7}".to_string()));
}

#[test]
fn a_failing_load_reports_its_error() {
    let error = App::try_load("syntax error here").err().unwrap();
    assert!(error.starts_with("SyntaxError: "), "{error}");
}

#[test]
fn every_failure_of_a_call_is_reported_and_the_rest_still_runs() {
    let mut app = App::load(
        r#"
        batch(["createElement", 1, "text"], ["setRoot", 1]);
        let first = true;
        __gpuix.setEventHandler(() => {
          if (!first) return true;
          first = false;
          queueMicrotask(() => { throw new RangeError("from a microtask"); });
          Promise.reject(new Error("never handled"));
          reportError(new Error("reported"));
          queueMicrotask(() => batch(["setText", 1, "still ran"]));
          return true;
        });
        "#,
    );
    let error = app.click(1).unwrap_err();
    let reported = error.find("Error: reported").unwrap();
    let microtask = error.find("RangeError: from a microtask").unwrap();
    let rejection = error
        .find("Uncaught (in promise) Error: never handled")
        .unwrap();
    assert!(reported < microtask && microtask < rejection, "{error}");
    assert_eq!(app.text(1), Some("still ran"));
    // The failures were reported once; the next event starts clean.
    assert_eq!(app.click(1), Ok(true));
}

#[test]
fn a_rejection_handled_before_the_microtasks_run_out_is_not_reported() {
    let mut app = App::load(
        r#"
        __gpuix.setEventHandler(() => {
          const failed = Promise.reject(new Error("handled later"));
          queueMicrotask(() => failed.catch(() => console.log("caught")));
          return true;
        });
        "#,
    );
    assert_eq!(app.click(1), Ok(true));
    assert_eq!(app.log(), ["caught"]);
}

#[test]
fn a_failing_timer_does_not_stop_the_others() {
    let mut app = App::load(
        r#"
        setTimeout(() => { throw new Error("first failed"); });
        setTimeout(() => console.log("second ran"));
        "#,
    );
    let error = app.advance_to(0.0).unwrap_err();
    assert!(error.starts_with("Error: first failed"), "{error}");
    assert_eq!(app.log(), ["second ran"]);
}

#[test]
fn console_formats_values_and_keeps_levels() {
    let app = App::load(
        r#"
        const cyclic = { name: "loop" };
        cyclic.self = cyclic;
        console.log("plain", 1, true, null, undefined, 2n);
        console.info({ a: [1, "two"], m: new Map([[1, "x"]]), s: new Set([3]) });
        console.warn(cyclic);
        console.error(function named() {}, () => {});
        console.debug(new Date(0));
        "#,
    );
    assert_eq!(
        app.printed(),
        [
            (
                ConsoleLevel::Log,
                "plain 1 true null undefined 2n".to_string()
            ),
            (
                ConsoleLevel::Info,
                r#"{a: [1, "two"], m: Map(1) {1 => "x"}, s: Set(1) {3}}"#.to_string()
            ),
            (
                ConsoleLevel::Warn,
                r#"{name: "loop", self: [Circular]}"#.to_string()
            ),
            (
                ConsoleLevel::Error,
                "[Function named] [Function (anonymous)]".to_string()
            ),
            (ConsoleLevel::Debug, "1970-01-01T00:00:00.000Z".to_string()),
        ]
    );
}

#[test]
fn structured_clone_copies_plain_data() {
    let app = App::load(
        r#"
        const shared = { n: 1 };
        const source = {
          list: [shared, shared],
          map: new Map([[shared, new Set(["a"])]]),
          date: new Date(5),
          pattern: /x+/gi,
          bytes: new Uint8Array([1, 2, 3]).subarray(1),
          error: new TypeError("bad", { cause: shared }),
        };
        source.self = source;
        const copy = structuredClone(source);
        console.log(String([
          copy !== source,
          copy.self === copy,
          copy.list[0] !== shared,
          copy.list[0] === copy.list[1],
          copy.map.get(copy.list[0]).has("a"),
          copy.date.getTime() === 5,
          copy.pattern.source === "x+" && copy.pattern.flags === "gi",
          copy.bytes.join() === "2,3" && copy.bytes.buffer !== source.bytes.buffer,
          copy.error instanceof TypeError && copy.error.message === "bad",
          copy.error.cause === copy.list[0],
        ]));
        for (const value of [() => {}, { nested: Symbol("s") }, new (class Thing {})()]) {
          try {
            structuredClone(value);
            console.log("cloned");
          } catch (error) {
            console.log(error.name);
          }
        }
        "#,
    );
    assert_eq!(
        app.log(),
        [
            "true,true,true,true,true,true,true,true,true,true",
            "DataCloneError",
            "DataCloneError",
            "DataCloneError",
        ]
    );
}

#[test]
fn the_tree_goes_back_to_its_owner_after_each_call() {
    let mut app = App::load(r#"batch(["createElement", 1, "div"], ["setRoot", 1]);"#);
    assert_eq!(app.tree.root_id, Some(1));
    // A failed call hands it back too.
    let mut other = App::load(r#"__gpuix.setEventHandler(() => { throw new Error("no"); });"#);
    std::mem::swap(&mut app.tree, &mut other.tree);
    other.click(1).unwrap_err();
    assert_eq!(other.tree.root_id, Some(1));
    assert!(app.click(1).is_ok());
}

#[test]
fn dropping_a_runtime_with_pending_work_is_safe() {
    let app = App::load(
        r#"
        setInterval(() => {}, 10);
        globalThis.pending = new Promise(() => {});
        __gpuix.setEventHandler(() => true);
        "#,
    );
    drop(app);
}

#[test]
fn out_of_range_delays_are_clamped_as_node_does() {
    let mut app = App::load(
        r#"
        setTimeout(() => console.log("huge"), Number.MAX_VALUE);
        setTimeout(() => console.log("infinite"), Infinity);
        setTimeout(() => console.log("negative"), -5);
        setTimeout(() => console.log("not a number"), "soon");
        "#,
    );
    assert_eq!(app.js.next_timer(), Ok(Some(Duration::ZERO)));
    app.advance_to(0.0).unwrap();
    assert_eq!(app.log(), ["negative", "not a number"]);
    assert_eq!(app.js.next_timer(), Ok(Some(Duration::from_millis(1))));
    app.advance_to(1.0).unwrap();
    assert_eq!(app.log(), ["negative", "not a number", "huge", "infinite"]);
}

#[test]
fn a_zero_interval_runs_once_a_millisecond() {
    let mut app = App::load(r#"setInterval(() => console.log("tick"), 0);"#);
    app.advance_to(1.0).unwrap();
    app.advance_to(1.0).unwrap();
    assert_eq!(app.log(), ["tick"]);
    assert_eq!(app.js.next_timer(), Ok(Some(Duration::from_millis(1))));
}

#[test]
fn deeply_nested_timeouts_wait_at_least_four_milliseconds() {
    let mut app = App::load(
        r#"
        let depth = 0;
        const again = () => {
          depth += 1;
          console.log(String(depth));
          setTimeout(again, 0);
        };
        setTimeout(again, 0);
        "#,
    );
    // The first timer and five nested ones run without a delay.
    for _ in 0..6 {
        app.advance_to(0.0).unwrap();
    }
    assert_eq!(app.log().len(), 6);
    app.advance_to(0.0).unwrap();
    assert_eq!(app.log().len(), 6);
    assert_eq!(app.js.next_timer(), Ok(Some(Duration::from_millis(4))));
}

#[test]
fn an_interval_cleared_by_another_timer_of_the_same_round_stops() {
    let mut app = App::load(
        r#"
        const interval = setInterval(() => console.log("interval"), 10);
        setTimeout(() => clearInterval(interval), 10);
        "#,
    );
    app.advance_to(10.0).unwrap();
    app.advance_to(30.0).unwrap();
    assert_eq!(app.log(), ["interval"]);
    assert_eq!(app.js.next_timer(), Ok(None));
}

#[test]
fn an_interval_is_due_again_one_interval_after_it_ran() {
    let mut app = App::load(r#"setInterval(() => {}, 20);"#);
    // Run late: the next one is due 20 ms after this run, not at 40.
    app.advance_to(35.0).unwrap();
    app.time.set(36.0);
    assert_eq!(app.js.next_timer(), Ok(Some(Duration::from_millis(19))));
}

#[test]
fn rejections_inside_jobs_and_async_functions_are_reported() {
    let mut app = App::load(
        r#"
        __gpuix.setEventHandler(() => {
          Promise.resolve().then(() => { throw new Error("in a job"); });
          (async () => {
            await null;
            throw new Error("in an async function");
          })();
          (async () => {
            try {
              await Promise.reject(new Error("caught"));
            } catch {}
          })();
          return true;
        });
        "#,
    );
    let error = app.click(1).unwrap_err();
    assert!(
        error.contains("Uncaught (in promise) Error: in a job"),
        "{error}"
    );
    assert!(
        error.contains("Uncaught (in promise) Error: in an async function"),
        "{error}"
    );
    assert!(!error.contains("Error: caught"), "{error}");
}

#[test]
fn a_rejection_handled_in_a_later_call_is_reported_once() {
    let mut app = App::load(
        r#"
        let failed;
        __gpuix.setEventHandler((event) => {
          if (event.eventType === "fail") failed = Promise.reject(new Error("late"));
          else failed.catch(() => console.log("handled"));
          return true;
        });
        "#,
    );
    assert!(app.dispatch(event(1, "fail")).unwrap_err().contains("late"));
    assert_eq!(app.click(1), Ok(true));
    assert_eq!(app.log(), ["handled"]);
}

#[test]
fn a_rejection_made_while_describing_one_fails_the_same_call() {
    let mut app = App::load(
        r#"
        __gpuix.setEventHandler(() => {
          const error = new Error("outer");
          Object.defineProperty(error, "stack", {
            get() {
              Promise.reject(new Error("from the getter"));
              return "the stack";
            },
          });
          Promise.reject(error);
          return true;
        });
        "#,
    );
    let error = app.click(1).unwrap_err();
    assert!(error.contains("from the getter"), "{error}");
    assert_eq!(app.click(1).map_err(|_| ()), Err(()));
    drop(app);
}

#[test]
fn a_panic_in_a_native_function_fails_the_call_even_if_js_catches_it() {
    let mut tree = RetainedTree::new();
    let mut js = QuickJsRuntime::load(
        r#"
        __gpuix.setEventHandler(() => {
          try {
            console.log("boom");
          } catch {}
          return true;
        });
        "#,
        "app.js",
        &mut tree,
        QuickJsOptions {
            clock: Box::new(|| 0.0),
            console: Box::new(|_, _| panic!("the sink broke")),
        },
    )
    .unwrap();
    let error = js
        .dispatch_event(&event(1, "click"), &mut tree)
        .unwrap_err();
    assert_eq!(error, "GPUIX panicked in console: the sink broke");
    // Nothing is left parked for the next call.
    let next = js
        .dispatch_event(&event(1, "click"), &mut tree)
        .unwrap_err();
    assert_eq!(next, "GPUIX panicked in console: the sink broke");
}

#[test]
fn console_fills_in_format_specifiers() {
    let app = App::load(
        r#"
        console.log("%s is %d (%i) or %f, %j %o %c%%", "a", 4.5, 4.5, "1.5x", { b: 1 }, [2], "color: red", "extra");
        console.log("%s missing %s", "one");
        console.log("%s", { nested: "quoted" });
        "#,
    );
    assert_eq!(
        app.log(),
        [
            r#"a is 4.5 (4) or 1.5, {"b":1} [2] % extra"#,
            "one missing %s",
            r#"{nested: "quoted"}"#,
        ]
    );
}

#[test]
fn console_reads_no_getters_or_proxy_traps_and_never_throws() {
    let app = App::load(
        r#"
        let trapped = false;
        const store = new Proxy({ count: 1 }, {
          get(target, key) { trapped = true; return target[key]; },
          ownKeys(target) { trapped = true; return Reflect.ownKeys(target); },
          getOwnPropertyDescriptor(target, key) {
            trapped = true;
            return Reflect.getOwnPropertyDescriptor(target, key);
          },
        });
        const { proxy, revoke } = Proxy.revocable({}, {});
        revoke();
        console.log({ get loud() { throw new Error("read"); }, store, proxy });
        console.log(String(trapped));
        class Point { constructor() { this.x = 1; } }
        console.log(new Point(), Object.create(null), { a: { b: { c: { d: 1 } } } });
        const error = new Error("outer", { cause: new Error("inner") });
        error.stack = "    at somewhere\n";
        console.log(error);
        "#,
    );
    assert_eq!(
        app.log(),
        [
            "{loud: [Getter], store: {count: 1}, proxy: [Revoked Proxy]}",
            "false",
            "Point {x: 1} [Object: null prototype] {} {a: {b: {c: [Object]}}}",
            "Error: outer\n    at somewhere\nCaused by: Error: inner\n    at <eval> (app.js:18:55)",
        ]
    );
}

#[test]
fn console_has_nodes_other_methods() {
    let mut app = App::load(
        r#"
        console.assert(true, "hidden");
        console.assert(false, "shown", 1);
        console.group("outer");
        console.log("inside");
        console.groupEnd();
        console.count();
        console.count();
        console.countReset();
        console.count();
        console.time("t");
        __gpuix.setEventHandler(() => {
          console.timeEnd("t");
          console.timeEnd("t");
          return true;
        });
        console.dir({ a: 1 });
        console.table([1]);
        "#,
    );
    app.time.set(2.5);
    app.click(1).unwrap();
    assert_eq!(
        app.printed(),
        [
            (ConsoleLevel::Error, "Assertion failed: shown 1".to_string()),
            (ConsoleLevel::Log, "outer".to_string()),
            (ConsoleLevel::Log, "  inside".to_string()),
            (ConsoleLevel::Info, "default: 1".to_string()),
            (ConsoleLevel::Info, "default: 2".to_string()),
            (ConsoleLevel::Info, "default: 1".to_string()),
            (ConsoleLevel::Log, "{a: 1}".to_string()),
            (ConsoleLevel::Log, "[1]".to_string()),
            (ConsoleLevel::Info, "t: 2.500ms".to_string()),
            (ConsoleLevel::Warn, "No such label 't'".to_string()),
        ]
    );
}

#[test]
fn structured_clone_keeps_odd_keys_buffers_and_holes() {
    let app = App::load(
        r#"
        const tricky = structuredClone(JSON.parse('{"__proto__": {"polluted": 1}}'));
        const buffer = new ArrayBuffer(4);
        const views = structuredClone([new Uint8Array(buffer), new DataView(buffer, 1, 2)]);
        const sparse = structuredClone([1, , 3]);
        const boxed = structuredClone(new Number(3));
        let fake;
        try {
          structuredClone(Object.create(Map.prototype));
        } catch (error) {
          fake = error.name;
        }
        let transfer;
        try {
          structuredClone(buffer, { transfer: [buffer] });
        } catch (error) {
          transfer = error.name;
        }
        console.log(String([
          tricky.polluted === undefined && Object.getPrototypeOf(tricky) === Object.prototype,
          Object.hasOwn(tricky, "__proto__") && tricky["__proto__"].polluted === 1,
          views[0].buffer === views[1].buffer && views[1].byteOffset === 1 && views[1].byteLength === 2,
          sparse.length === 3 && !(1 in sparse),
          boxed instanceof Number && boxed.valueOf() === 3,
          fake,
          transfer,
        ]));
        "#,
    );
    assert_eq!(
        app.log(),
        ["true,true,true,true,true,DataCloneError,DataCloneError"]
    );
}

#[test]
fn the_event_handler_can_be_cleared_and_its_result_is_a_boolean() {
    let mut app = App::load(
        r#"
        __gpuix.setEventHandler(() => "yes");
        __gpuix.setEventHandler((event) => (event.eventType === "clear" ? __gpuix.setEventHandler(undefined) : 1));
        "#,
    );
    assert_eq!(app.click(1), Ok(true));
    assert_eq!(app.dispatch(event(1, "clear")), Ok(false));
    assert_eq!(app.click(1), Ok(false));
}

#[test]
fn writing_to_the_host_object_does_not_change_the_runtime() {
    let mut app = App::load(
        r#"
        __gpuix.now = () => 999;
        __gpuix.dueTimers = () => { throw new Error("replaced"); };
        setTimeout(() => console.log(String(performance.now())), 5);
        "#,
    );
    app.advance_to(5.0).unwrap();
    assert_eq!(app.log(), ["5"]);
}

// ── Modules ──────────────────────────────────────────────────────────────

/// A directory of app files under the system temp directory, removed on drop.
struct AppDir(PathBuf);

impl AppDir {
    fn new(files: &[(&str, &str)]) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let unique = format!(
            "gpuix-quickjs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let dir = Self(std::env::temp_dir().join(unique));
        for (name, source) in files {
            let path = dir.0.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, source).unwrap();
        }
        dir
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for AppDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl App {
    /// Run timers that are due now until none is: React's scheduler works
    /// in `setImmediate` slices.
    fn settle(&mut self) {
        while self.js.next_timer().unwrap() == Some(Duration::ZERO) {
            self.js.run_timers(&mut self.tree).unwrap();
        }
    }

    fn test_id(&self, test_id: &str) -> u64 {
        self.tree
            .elements
            .values()
            .find(|element| element.test_id.as_deref() == Some(test_id))
            .unwrap_or_else(|| panic!("no element with test id {test_id:?}"))
            .id
    }

    fn texts(&self) -> Vec<String> {
        let mut texts: Vec<String> = self
            .tree
            .elements
            .values()
            .filter_map(|element| element.content.clone())
            .collect();
        texts.sort();
        texts
    }
}

#[test]
fn typescript_and_jsx_compile_to_plain_javascript() {
    let code = compile(
        "const size: number = 2;\nexport const view = <div style={{ width: size }}>{'hi'}</div>;\n",
        Path::new("view.tsx"),
    )
    .unwrap()
    .code;
    assert!(
        code.contains(r#"from "@gpuix/react/jsx-runtime""#),
        "{code}"
    );
    assert!(!code.contains(": number"), "{code}");
    assert!(compile("export const a = 1 as const;", Path::new("a.mts")).is_ok());
    for name in ["a.jsx", "a.js", "a.mjs"] {
        assert!(compile("export const a = <b />;", Path::new(name)).is_ok());
    }
    // TypeScript's own `<Type>value` casts stay unambiguous in `.ts`.
    assert!(compile("export const a = <number>1;", Path::new("a.ts")).is_ok());
}

#[test]
fn what_quickjs_cant_run_fails_the_compile_where_it_is() {
    let error = |source: &str| match compile(source, Path::new("app.ts")) {
        Ok(compiled) => panic!("{source:?} compiled to {}", compiled.code),
        Err(error) => error,
    };
    let decorator = error("const sealed = (c: unknown) => c;\n@sealed class A {}\n");
    assert!(
        decorator.starts_with("app.ts:2:1: Decorators aren't supported"),
        "{decorator}"
    );
    let accessor = error("class B { accessor x = 1 }");
    assert!(
        accessor.starts_with("app.ts:1:11: `accessor` fields aren't supported"),
        "{accessor}"
    );
    let require = error("import fs = require('fs')");
    assert!(
        require.starts_with("app.ts:1:1: `import ... = require(...)`"),
        "{require}"
    );
    assert!(require.contains("help: apps are ES modules"), "{require}");
    let export = error("export = 1");
    assert!(
        export.starts_with("app.ts:1:1: `export =` isn't supported"),
        "{export}"
    );
    // An alias of a namespace member is fine: it is plain TypeScript. So is
    // what TypeScript removes: `declare` blocks, abstract members and
    // type-only imports.
    for source in [
        "namespace N { export const x = 1 }\nimport x = N.x",
        "declare module \"untyped\" { const x: any; export = x }",
        "declare class D { accessor x: number }",
        "export declare namespace E { class F { accessor g: number } }",
        "abstract class C { abstract accessor x: number }",
        "import type fs = require(\"fs\")",
    ] {
        if let Err(error) = compile(source, Path::new("ok.ts")) {
            panic!("{source:?} failed: {error}");
        }
    }
}

#[test]
fn imports_stay_unless_marked_as_types() {
    let code = compile(
        r#"
        import unused from "./unused"
        import { used, type Shape } from "./used"
        import type { Only } from "./types"
        import "./effects"
        export const value: Shape | Only = used
        "#,
        Path::new("main.ts"),
    )
    .unwrap()
    .code;
    assert!(code.contains(r#"from "./unused""#), "{code}");
    assert!(code.contains(r#"import { used } from "./used""#), "{code}");
    assert!(code.contains(r#"import "./effects""#), "{code}");
    assert!(!code.contains("./types"), "{code}");
}

#[test]
fn compile_errors_give_the_file_line_and_column() {
    let error = compile("const a = 1;\nconst b = ;\n", Path::new("src/bad.ts"))
        .err()
        .unwrap();
    assert!(error.starts_with("src/bad.ts:2:11: "), "{error}");
    let error = compile("let a = 1;\nlet a = 2;\n", Path::new("twice.ts"))
        .err()
        .unwrap();
    assert_eq!(
        error,
        "twice.ts:1:5: Identifier `a` has already been declared\n  \
         twice.ts:1:5: `a` has already been declared here\n  \
         twice.ts:2:5: It can not be redeclared here"
    );
    for name in ["types.d.ts", "old.cjs", "data.json"] {
        let error = compile("", Path::new(name)).err().unwrap();
        assert!(error.contains("an app module must be"), "{error}");
    }
    // A byte-order mark isn't a column.
    let error = compile("\u{FEFF}const b = ;", Path::new("bom.ts"))
        .err()
        .unwrap();
    assert!(error.starts_with("bom.ts:1:11: "), "{error}");
    // Columns count UTF-16 code units, as editors do: the emoji is two.
    let error = compile("const a = \"😀\" + ;", Path::new("e.ts"))
        .err()
        .unwrap();
    assert!(error.starts_with("e.ts:1:18: "), "{error}");
}

#[test]
fn enums_namespaces_and_parameter_properties_run() {
    let dir = AppDir::new(&[(
        "main.ts",
        r#"
        enum Color { Red, Green = "g" }
        namespace Sizes { export const small = 1 }
        class Point { constructor(public x: number) {} }
        console.log(Color.Red, Color.Green, Sizes.small, new Point(3).x)
        "#,
    )]);
    let app = App::try_load_module(&dir.path("main.ts")).unwrap();
    assert_eq!(app.log(), ["0 g 1 3"]);
}

#[test]
fn stack_traces_point_at_the_source_lines() {
    let dir = AppDir::new(&[(
        "main.ts",
        r#"interface Shape {
  width: number
  height: number
}
type Size = number
export function fail(size: Size): never {
  throw new Error(`mapped ${size}`)
}
try { fail(1) } catch (error) { console.log((error as Error).stack) }
fail(2)
"#,
    )]);
    let main = std::fs::canonicalize(dir.path("main.ts")).unwrap();
    let main = main.display();
    let app = App::try_load_module(&dir.path("main.ts"));
    let error = app.err().unwrap();
    assert!(error.contains("Error: mapped 2"), "{error}");
    assert!(error.contains(&format!("at fail ({main}:7:")), "{error}");
    assert!(error.contains(&format!("({main}:10:")), "{error}");
}

#[test]
fn stack_traces_map_lines_and_columns_past_unusual_line_breaks() {
    // A line separator inside a template is one line to oxc but not to
    // QuickJS or an editor; a lone `\r` is one line to all three.
    let dir = AppDir::new(&[(
        "main.ts",
        "const a: string = `x\u{2028}y`\nconst b = `p\rq`\nfunction fail() {\n  const s = \"😀\"; throw new Error(a + b + s)\n}\nfail()\n",
    )]);
    let main = std::fs::canonicalize(dir.path("main.ts")).unwrap();
    let error = App::try_load_module(&dir.path("main.ts")).err().unwrap();
    // The lone `\r` makes the `throw` line 5. QuickJS puts a `new` call at
    // its last argument, `s`, here after the emoji's two UTF-16 units.
    assert!(
        error.contains(&format!("at fail ({}:5:43)", main.display())),
        "{error}"
    );
    assert!(
        error.contains(&format!("({}:7:1)", main.display())),
        "{error}"
    );
}

#[test]
fn stack_traces_keep_quickjs_frames_and_limits() {
    let app = App::try_load(
        r#"
        try { [1].map(() => { throw new Error("in map") }) }
        catch (error) { console.log(error.stack) }
        Error.stackTraceLimit = 1
        console.log(new Error("one").stack)
        Error.stackTraceLimit = 0
        console.log(new Error("none").stack)
        "#,
    )
    .unwrap();
    let log = app.log();
    assert!(log[0].contains("    at map (native)\n"), "{}", log[0]);
    assert_eq!(log[1].matches("    at ").count(), 1, "{}", log[1]);
    // quickjs-ng's stack holds only the frames, not the message.
    assert_eq!(log[2], "");
}

#[test]
fn a_stack_overflow_keeps_its_mapped_frames() {
    let dir = AppDir::new(&[(
        "main.ts",
        "type N = number\nfunction deep(n: N): N {\n  return deep(n + 1) + 1\n}\ndeep(0)\n",
    )]);
    let main = std::fs::canonicalize(dir.path("main.ts")).unwrap();
    let error = App::try_load_module(&dir.path("main.ts")).err().unwrap();
    assert!(
        error.contains("Maximum call stack size exceeded"),
        "{error}"
    );
    assert!(
        error.contains(&format!("at deep ({}:3:", main.display())),
        "{error}"
    );
}

#[test]
fn an_apps_own_stack_format_gets_source_positions() {
    let dir = AppDir::new(&[(
        "main.ts",
        r#"type Site = { getLineNumber(): number; getColumnNumber(): number }
Error.prepareStackTrace = (_error, sites: Site[]) =>
  sites.map((site) => `${site.getLineNumber()}:${site.getColumnNumber()}`).join(" ")
function make(): Error {
  return new Error("here")
}
console.log(make().stack)
"#,
    )]);
    let app = App::try_load_module(&dir.path("main.ts")).unwrap();
    // QuickJS puts `new Error(...)` at `Error`, and the call at `make`.
    assert_eq!(app.log(), ["5:14 7:13"]);
}

#[test]
fn a_bad_regex_fails_the_compile_where_it_is() {
    let error = compile("type T = number\nconst r = /[z-a]/\n", Path::new("re.ts"))
        .err()
        .unwrap();
    assert!(error.starts_with("re.ts:2:"), "{error}");
}

#[test]
fn a_long_compile_error_reaches_the_app_whole() {
    let dir = AppDir::new(&[("main.ts", "let a = 1;\nlet a = 2;\n")]);
    let error = App::try_load_module(&dir.path("main.ts")).err().unwrap();
    assert!(error.contains("SyntaxError: "), "{error}");
    assert!(error.contains("It can not be redeclared here"), "{error}");
}

#[test]
fn a_caught_dynamic_import_doesnt_fail_the_load() {
    let dir = AppDir::new(&[
        (
            "main.ts",
            r#"
            import("./missing").catch((error) => console.log(`missing: ${error.message}`))
            import("./broken").catch((error) => console.log(`broken: ${error.message}`))
            "#,
        ),
        ("broken.ts", "const = 1"),
    ]);
    let app = App::try_load_module(&dir.path("main.ts")).unwrap();
    let log = app.log();
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(
        log.iter()
            .any(|line| line.starts_with("missing: Cannot find")),
        "{log:?}"
    );
    assert!(
        log.iter().any(|line| line.contains("broken.ts:1:7")),
        "{log:?}"
    );
}

#[test]
fn an_app_cant_hide_a_failed_load_by_replacing_report_error() {
    let dir = AppDir::new(&[(
        "main.ts",
        "globalThis.reportError = () => {}\nthrow new Error('still reported')\n",
    )]);
    let error = App::try_load_module(&dir.path("main.ts")).err().unwrap();
    assert!(error.contains("still reported"), "{error}");
}

#[test]
fn a_react_app_renders_and_handles_clicks() {
    let dir = AppDir::new(&[
        (
            "main.tsx",
            r#"
            import { render } from "@gpuix/react"
            import { Counter } from "./counter"
            render(<Counter label="Count" />)
            "#,
        ),
        (
            "counter.tsx",
            r#"
            import { useState } from "react"
            export function Counter({ label }: { label: string }) {
              const [count, setCount] = useState(0)
              return (
                <div testId="counter" onClick={() => setCount((value) => value + 1)}>
                  <text>{`${label}: ${count}`}</text>
                </div>
              )
            }
            "#,
        ),
    ]);
    let mut app = App::try_load_module(&dir.path("main.tsx")).unwrap();
    app.settle();
    assert_eq!(app.texts(), ["Count: 0"]);
    let counter = app.test_id("counter");
    // A click commits in a microtask, before the call returns, and says
    // whether a handler took it.
    assert!(app.click(counter).unwrap());
    assert_eq!(app.texts(), ["Count: 1"]);
    assert!(!app.click(counter + 1000).unwrap());
    assert!(app.log().contains(&"[gpuix] mount complete".to_string()));
}

#[test]
fn render_passes_its_window_options_to_the_host() {
    let dir = AppDir::new(&[(
        "main.tsx",
        r#"
        import { render } from "@gpuix/react"
        render(<text>one</text>, { title: "Mail", width: 640, minHeight: 200, focus: false,
          windowBackground: "blurred", onKeyDown: () => {} })
        // A second render remounts in the same window.
        render(<text>two</text>, { title: "Ignored" })
        "#,
    )]);
    let mut app = App::try_load_module(&dir.path("main.tsx")).unwrap();
    app.settle();
    let options = app.js.window_options().unwrap();
    assert_eq!(options.title.as_deref(), Some("Mail"));
    assert_eq!(options.width, Some(640.0));
    assert_eq!(options.height, None);
    assert_eq!(options.min_height, Some(200.0));
    assert_eq!(options.focus, Some(false));
    assert_eq!(options.window_background.as_deref(), Some("blurred"));
    assert_eq!(app.texts(), ["two"]);
}

#[test]
fn a_renderer_initializes_once_with_valid_options() {
    let dir = AppDir::new(&[(
        "main.ts",
        r#"
        import { GpuixRenderer } from "@gpuix/native"
        const renderer = new GpuixRenderer()
        try { renderer.init({ width: "wide" } as any) } catch (error) { console.log(String(error)) }
        renderer.init()
        try { renderer.init() } catch (error) { console.log(String(error)) }
        "#,
    )]);
    let app = App::try_load_module(&dir.path("main.ts")).unwrap();
    let log = app.log();
    assert!(
        log[0].starts_with("TypeError: invalid window options: "),
        "{log:?}"
    );
    assert_eq!(log[1], "Error: Renderer is already initialized");
    assert_eq!(app.js.window_options().unwrap().title, None);
}

#[test]
fn the_host_hears_when_a_batch_changed_the_tree() {
    let dir = AppDir::new(&[(
        "main.tsx",
        r#"
        import { render } from "@gpuix/react"
        import { useState } from "react"
        function Counter() {
          const [count, setCount] = useState(0)
          return <div testId="counter" onClick={() => setCount(count + 1)}><text>{count}</text></div>
        }
        render(<Counter />)
        "#,
    )]);
    let mut app = App::try_load_module(&dir.path("main.tsx")).unwrap();
    app.settle();
    assert!(app.js.take_tree_changed());
    assert!(!app.js.take_tree_changed());
    let counter = app.test_id("counter");
    // A click on nothing commits nothing.
    assert!(!app.click(counter + 1000).unwrap());
    assert!(!app.js.take_tree_changed());
    assert!(app.click(counter).unwrap());
    assert!(app.js.take_tree_changed());
}

#[test]
fn imports_resolve_the_way_bun_finds_files() {
    let dir = AppDir::new(&[
        (
            "main.ts",
            r#"
            import { a } from "./lib/a.js"
            import { b } from "./lib/b"
            import { c } from "./lib"
            import { a as again } from "./lib/../lib/a.ts"
            import { useState } from "react"
            console.log(a, b, c, again === a, typeof useState)
            "#,
        ),
        ("lib/a.ts", "export const a = 'a' as string"),
        ("lib/b.tsx", "export const b = 'b'"),
        ("lib/index.mjs", "export const c = 'c'"),
    ]);
    let app = App::try_load_module(&dir.path("main.ts")).unwrap();
    assert_eq!(app.log(), ["a b c true function"]);
}

#[test]
fn an_import_without_an_extension_tries_them_in_buns_order() {
    let dir = AppDir::new(&[
        (
            "main.ts",
            r#"
            import { from as exact } from "./exact.js"
            import { from as first } from "./first"
            import { from as second } from "./second"
            console.log(exact, first, second)
            "#,
        ),
        ("exact.js", "export const from = 'js'"),
        ("exact.ts", "export const from = 'ts'"),
        ("first.ts", "export const from = 'ts'"),
        ("first.tsx", "export const from = 'tsx'"),
        ("second.ts", "export const from = 'ts'"),
        ("second.jsx", "export const from = 'jsx'"),
    ]);
    let app = App::try_load_module(&dir.path("main.ts")).unwrap();
    assert_eq!(app.log(), ["js tsx jsx"]);
}

#[test]
fn importing_anything_else_fails_the_load() {
    let dir = AppDir::new(&[
        ("package.ts", "import 'lodash'"),
        ("node.ts", "import 'node:fs'"),
        ("missing.ts", "import './nowhere'"),
        (
            "json.ts",
            "import data from './data.json' with { type: 'json' }",
        ),
        ("data.json", "{}"),
        ("broken.ts", "import './syntax'"),
        ("syntax.ts", "export const a = 1;\nexport const = 2;\n"),
    ]);
    let error = |name: &str| match App::try_load_module(&dir.path(name)) {
        Ok(app) => panic!("{name} loaded: {:?}", app.log()),
        Err(error) => error,
    };
    let package = error("package.ts");
    assert!(package.contains(r#"Cannot import "lodash""#), "{package}");
    assert!(package.contains("@gpuix/react/jsx-runtime"), "{package}");
    assert!(error("node.ts").contains(r#"Cannot import "node:fs""#));
    assert!(error("missing.ts").contains(r#"Cannot find "./nowhere""#));
    assert!(error("json.ts").contains("aren't supported"));
    let syntax = error("broken.ts");
    assert!(syntax.contains("syntax.ts:2:"), "{syntax}");
    let entry = error("absent.ts");
    assert!(entry.contains("Cannot find /"), "{entry}");
    assert!(!entry.contains("imported from"), "{entry}");
}

#[test]
fn a_module_failing_after_top_level_await_fails_that_call() {
    let dir = AppDir::new(&[(
        "main.ts",
        r#"
        await new Promise((resolve) => setTimeout(resolve, 10))
        throw new Error("after the wait")
        "#,
    )]);
    let mut app = App::try_load_module(&dir.path("main.ts")).unwrap();
    let error = app.advance_to(10.0).unwrap_err();
    assert!(error.contains("Error: after the wait"), "{error}");
}
