// The browser and Node globals GPUIX's React and Solid packages expect, on
// top of what quickjs-ng has (`queueMicrotask`, promises, `Date`). Runs once
// per context, before the app, with `__gpuix` already installed by Rust:
//
//   __gpuix.now()                  milliseconds since the runtime started
//   __gpuix.print(level, message)  console output
//   __gpuix.reportError(message)   an error the app reported but didn't throw
//   __gpuix.proxyTarget(value)     a proxy's target, or undefined
//   __gpuix.init(json)             the window options `render()` passes
//   __gpuix.process                { env, argv, platform } for `process`
//   __gpuix.gc()                   a full garbage collection
//   __gpuix.renderer.applyBatch(json)
//   __gpuix.renderer.window(method, ...args)  a window method (`window.rs`)
//   __gpuix.renderer.promptForPaths(options)   a promise of the paths picked
//
// It adds what Rust calls back into: `dispatchEvent`, `dueTimers`,
// `runTimer`, `nextTimer` and `describe`. It keeps its own references to the
// native functions, so an app that writes to `__gpuix` changes nothing here.
(() => {
  "use strict";
  const host = globalThis.__gpuix;
  const { now, print, reportError, proxyTarget } = host;

  // ── Timers ─────────────────────────────────────────────────────────
  // Rust asks for the timers due at the current time, then runs them one at
  // a time with the microtask queue drained after each, as a browser does.
  // A timer queued while they run waits for the next round, even with no
  // delay, so a timer that keeps re-arming can't starve the window.
  const timers = new Map();
  let nextTimerId = 1;
  // The timer whose callback is running, if any.
  let running;

  // Node's limit. Longer delays, Infinity included, become 1 ms there too.
  const TIMEOUT_MAX = 2 ** 31 - 1;
  // As in browsers: past this many timers that each queued the next, a delay
  // is at least 4 ms, so a timer that keeps re-arming itself doesn't keep the
  // host busy.
  const MAX_NESTING = 5;
  const NESTED_DELAY = 4;

  const schedule = (callback, delay, args, kind) => {
    if (typeof callback !== "function") {
      throw new TypeError("The timer callback must be a function");
    }
    let wait = Number(delay);
    if (!(wait >= 0)) wait = 0;
    if (wait > TIMEOUT_MAX) wait = 1;
    const nesting = running === undefined ? 0 : running.nesting + 1;
    if (kind !== "immediate" && nesting > MAX_NESTING) wait = Math.max(wait, NESTED_DELAY);
    // An interval of 0 would be due again on every round.
    if (kind === "interval") wait = Math.max(wait, 1);
    const id = nextTimerId++;
    timers.set(id, {
      at: now() + wait,
      callback,
      args,
      interval: kind === "interval" ? wait : undefined,
      nesting,
    });
    return id;
  };
  const clear = (id) => {
    timers.delete(id);
  };

  globalThis.setTimeout = (callback, delay, ...args) => schedule(callback, delay, args, "timeout");
  globalThis.setInterval = (callback, delay, ...args) =>
    schedule(callback, delay, args, "interval");
  globalThis.setImmediate = (callback, ...args) => schedule(callback, 0, args, "immediate");
  globalThis.clearTimeout = clear;
  globalThis.clearInterval = clear;
  globalThis.clearImmediate = clear;

  host.dueTimers = () => {
    const time = now();
    const due = [];
    for (const [id, timer] of timers) {
      if (timer.at <= time) due.push([timer.at, id]);
    }
    // Map order is creation order, and the sort is stable, so that breaks
    // ties between equal times.
    due.sort((left, right) => left[0] - right[0]);
    return due.map(([, id]) => id);
  };
  host.runTimer = (id) => {
    const timer = timers.get(id);
    // An earlier timer of the same round cleared it.
    if (timer === undefined) return;
    if (timer.interval === undefined) timers.delete(id);
    else timer.at = now() + timer.interval;
    running = timer;
    try {
      timer.callback(...timer.args);
    } finally {
      running = undefined;
    }
  };
  // Milliseconds until the next timer is due (0 when one is due already), or
  // -1 when none is queued.
  host.nextTimer = () => {
    let next = Infinity;
    for (const timer of timers.values()) next = Math.min(next, timer.at);
    return next === Infinity ? -1 : Math.max(0, next - now());
  };

  // ── Events ─────────────────────────────────────────────────────────
  // The JS side of GPUIX registers one handler, like the callback a Node app
  // passes to `new GpuixRenderer(...)`. Its return value says whether a
  // handler took the event.
  let eventHandler;
  let trayHandler;
  host.setTrayHandler = (handler) => {
    if (typeof handler !== "function") throw new TypeError("Tray handler must be a function");
    trayHandler = handler;
  };
  host.dispatchTrayEvent = (action) => trayHandler?.(action);
  host.setEventHandler = (handler) => {
    if (handler !== undefined && typeof handler !== "function") {
      throw new TypeError("The event handler must be a function");
    }
    eventHandler = handler;
  };
  host.dispatchEvent = (json) =>
    eventHandler === undefined ? false : Boolean(eventHandler(JSON.parse(json)));

  // ── Console ────────────────────────────────────────────────────────
  // Like Node's console: apart from an error's `stack` it reads no getters
  // and runs no proxy traps (a proxy prints as its target), so logging a
  // Solid store doesn't subscribe to it, and printing never throws.
  const MAX_DEPTH = 2;

  const errorText = (error, seen, depth) => {
    let text = String(error);
    // quickjs-ng keeps the stack behind a getter on Error.prototype. Node
    // reads it too.
    const stack = error.stack;
    if (typeof stack === "string" && stack !== "") text += `\n${stack.trimEnd()}`;
    const cause = Object.getOwnPropertyDescriptor(error, "cause");
    if (cause && "value" in cause) {
      text += `\nCaused by: ${describe(cause.value, seen, depth + 1)}`;
    }
    return text;
  };

  const property = (object, key, seen, depth) => {
    const descriptor = Object.getOwnPropertyDescriptor(object, key);
    if (descriptor.get && descriptor.set) return "[Getter/Setter]";
    if (descriptor.get) return "[Getter]";
    if (descriptor.set) return "[Setter]";
    return describe(descriptor.value, seen, depth + 1);
  };

  const className = (object) => {
    const prototype = Object.getPrototypeOf(object);
    if (prototype === null) return "[Object: null prototype] ";
    if (prototype === Object.prototype) return "";
    const constructor = Object.getOwnPropertyDescriptor(prototype, "constructor");
    const name = typeof constructor?.value === "function" ? constructor.value.name : "";
    return name ? `${name} ` : "";
  };

  const describeObject = (value, seen, depth) => {
    if (value instanceof Error) return errorText(value, seen, depth);
    if (value instanceof Date) {
      return Number.isNaN(value.getTime()) ? "Invalid Date" : value.toISOString();
    }
    if (value instanceof RegExp) return String(value);
    if (seen.has(value)) return "[Circular]";
    if (depth > MAX_DEPTH) return Array.isArray(value) ? "[Array]" : "[Object]";
    seen.add(value);
    try {
      if (Array.isArray(value)) {
        const items = Object.keys(value).map((key) => property(value, key, seen, depth));
        return `[${items.join(", ")}]`;
      }
      if (value instanceof Map) {
        const entries = [...value].map(
          ([key, item]) => `${describe(key, seen, depth + 1)} => ${describe(item, seen, depth + 1)}`
        );
        return `Map(${value.size}) {${entries.join(", ")}}`;
      }
      if (value instanceof Set) {
        const items = [...value].map((item) => describe(item, seen, depth + 1));
        return `Set(${value.size}) {${items.join(", ")}}`;
      }
      const entries = Object.keys(value).map(
        (key) => `${key}: ${property(value, key, seen, depth)}`
      );
      return `${className(value)}{${entries.join(", ")}}`;
    } finally {
      seen.delete(value);
    }
  };

  const describe = (value, seen = new Set(), depth = 0) => {
    try {
      if (typeof value === "string") return depth === 0 ? value : JSON.stringify(value);
      if (typeof value === "bigint") return `${value}n`;
      if (typeof value === "symbol") return value.toString();
      if (typeof value === "function") return `[Function ${value.name || "(anonymous)"}]`;
      if (typeof value !== "object" || value === null) return String(value);
      let target = value;
      for (let inner = proxyTarget(target); inner !== undefined; inner = proxyTarget(target)) {
        target = inner;
      }
      // A revoked proxy has a null target.
      if (target === null) return "[Revoked Proxy]";
      return describeObject(target, seen, depth);
    } catch {
      return "[could not be printed]";
    }
  };

  // Node's format specifiers, in the first argument when it is a string.
  const format = (values) => {
    const parts = [];
    let rest = 0;
    if (typeof values[0] === "string") {
      rest = 1;
      parts.push(
        values[0].replace(/%([sdifjoOc%])/g, (match, specifier) => {
          if (specifier === "%") return "%";
          if (rest >= values.length) return match;
          const value = values[rest++];
          switch (specifier) {
            case "s":
              return typeof value === "string" ? value : describe(value, new Set(), 1);
            case "d":
            case "i": {
              if (typeof value === "bigint") return `${value}n`;
              if (typeof value === "symbol") return "NaN";
              const number = Number(value);
              return String(specifier === "i" ? Math.trunc(number) : number);
            }
            case "f":
              return typeof value === "symbol" ? "NaN" : String(Number.parseFloat(value));
            case "j":
              try {
                return JSON.stringify(value);
              } catch {
                return "[Circular]";
              }
            case "c":
              return "";
            default:
              return describe(value, new Set(), 1);
          }
        })
      );
    }
    for (let index = rest; index < values.length; index++) parts.push(describe(values[index]));
    return parts.join(" ");
  };

  let indent = "";
  const write = (level, values) => {
    const message = format(values);
    print(level, indent === "" ? message : message.replace(/^/gm, indent));
  };
  const counts = new Map();
  const timings = new Map();
  const elapsed = (label) => `${label}: ${(now() - timings.get(label)).toFixed(3)}ms`;
  globalThis.console = {
    log: (...values) => write("log", values),
    info: (...values) => write("info", values),
    debug: (...values) => write("debug", values),
    warn: (...values) => write("warn", values),
    error: (...values) => write("error", values),
    dir: (value) => write("log", [describe(value)]),
    table: (value) => write("log", [describe(value)]),
    trace: (...values) => {
      const stack = String(new Error().stack ?? "").split("\n").slice(1).join("\n").trimEnd();
      write("debug", [`Trace: ${format(values)}`, ...(stack ? [`\n${stack}`] : [])]);
    },
    assert: (condition, ...values) => {
      if (condition) return;
      write("error", values.length === 0 ? ["Assertion failed"] : ["Assertion failed:", ...values]);
    },
    group: (...values) => {
      if (values.length > 0) write("log", values);
      indent += "  ";
    },
    groupCollapsed: (...values) => console.group(...values),
    groupEnd: () => {
      indent = indent.slice(2);
    },
    count: (label = "default") => {
      const count = (counts.get(label) ?? 0) + 1;
      counts.set(label, count);
      write("info", [`${label}: ${count}`]);
    },
    countReset: (label = "default") => {
      counts.delete(label);
    },
    time: (label = "default") => {
      timings.set(label, now());
    },
    timeLog: (label = "default", ...values) => {
      if (!timings.has(label)) return write("warn", [`No such label '${label}'`]);
      write("info", [elapsed(label), ...values]);
    },
    timeEnd: (label = "default") => {
      if (!timings.has(label)) return write("warn", [`No such label '${label}'`]);
      write("info", [elapsed(label)]);
      timings.delete(label);
    },
  };
  globalThis.reportError = (error) => reportError(describe(error));
  // How Rust words an exception it caught.
  host.describe = (value) => describe(value);

  // ── performance ────────────────────────────────────────────────────
  // quickjs-ng has its own clock. Use the runtime's, which timers use too.
  globalThis.performance = { now, timeOrigin: Date.now() - now() };

  // ── process ────────────────────────────────────────────────────────
  // What apps written for Node and Bun read to learn where they run: the
  // environment, the command line (the program, then the entry file) and
  // the platform. It is a read-only snapshot; nothing in it reaches the
  // system. A partial `process` would break code that checks
  // `typeof process` and then uses `argv` or `platform`, so all three are here.
  {
    const { env, argv, platform } = host.process;
    globalThis.process = Object.freeze({
      env: Object.freeze({ ...env }),
      argv: Object.freeze([...argv]),
      platform,
    });
  }

  // ── gc ─────────────────────────────────────────────────────────────
  // Node's under `--expose-gc`, which `idle-gc.ts` runs once the app goes
  // quiet. QuickJS frees most objects by reference counting; this collects
  // the cycles that leaves behind.
  {
    const { gc } = host;
    globalThis.gc = () => gc();
  }

  // ── structuredClone ────────────────────────────────────────────────
  // Plain data only: primitives and their boxed forms, plain objects,
  // arrays, Map, Set, Date, RegExp, errors, ArrayBuffers, typed arrays and
  // DataViews, with shared and cyclic references kept. Anything else throws,
  // as the platform's version does. Transferring is not supported.
  const cloneError = (message) => {
    const error = new Error(message);
    error.name = "DataCloneError";
    return error;
  };
  const TypedArray = Object.getPrototypeOf(Uint8Array);
  const ERRORS = { EvalError, RangeError, ReferenceError, SyntaxError, TypeError, URIError };
  const BOXES = [
    [Number, Number.prototype.valueOf],
    [String, String.prototype.valueOf],
    [Boolean, Boolean.prototype.valueOf],
    [BigInt, BigInt.prototype.valueOf],
  ];
  // Set own properties by definition, so a `__proto__` key stays a key.
  const define = (object, key, value) =>
    Object.defineProperty(object, key, {
      value,
      writable: true,
      enumerable: true,
      configurable: true,
    });
  globalThis.structuredClone = (value, options) => {
    if (options?.transfer !== undefined && [...options.transfer].length > 0) {
      throw cloneError("structuredClone cannot transfer values");
    }
    const copies = new Map();
    const remember = (input, output) => {
      copies.set(input, output);
      return output;
    };
    const copy = (input) => {
      if (typeof input === "function" || typeof input === "symbol") {
        throw cloneError(`${String(input)} could not be cloned`);
      }
      if (typeof input !== "object" || input === null) return input;
      if (proxyTarget(input) !== undefined) throw cloneError("A Proxy could not be cloned");
      const existing = copies.get(input);
      if (existing !== undefined) return existing;

      for (const [Box, valueOf] of BOXES) {
        if (input instanceof Box) return remember(input, Object(valueOf.call(input)));
      }
      if (input instanceof Date) return remember(input, new Date(input.getTime()));
      if (input instanceof RegExp) return remember(input, new RegExp(input.source, input.flags));
      if (input instanceof ArrayBuffer) return remember(input, input.slice(0));
      if (input instanceof TypedArray || input instanceof DataView) {
        const Kind = input.constructor;
        const buffer = copy(input.buffer);
        const length = input instanceof DataView ? input.byteLength : input.length;
        return remember(input, new Kind(buffer, input.byteOffset, length));
      }
      if (input instanceof Map) {
        const output = remember(input, new Map());
        for (const [key, item] of Map.prototype.entries.call(input)) {
          output.set(copy(key), copy(item));
        }
        return output;
      }
      if (input instanceof Set) {
        const output = remember(input, new Set());
        for (const item of Set.prototype.values.call(input)) output.add(copy(item));
        return output;
      }
      if (input instanceof Error) {
        const Kind = Object.hasOwn(ERRORS, input.name) ? ERRORS[input.name] : Error;
        const output = remember(input, new Kind(input.message));
        if (input.stack !== undefined) output.stack = String(input.stack);
        if ("cause" in input) output.cause = copy(input.cause);
        return output;
      }
      if (Array.isArray(input)) {
        const output = remember(input, new Array(input.length));
        for (const key of Object.keys(input)) define(output, key, copy(input[key]));
        return output;
      }
      const prototype = Object.getPrototypeOf(input);
      if (prototype !== Object.prototype && prototype !== null) {
        throw cloneError(`${Object.prototype.toString.call(input)} could not be cloned`);
      }
      const output = remember(input, {});
      for (const key of Object.keys(input)) define(output, key, copy(input[key]));
      return output;
    };
    try {
      return copy(value);
    } catch (error) {
      // A Map or Set method on something that only claims to be one.
      if (error instanceof TypeError) throw cloneError(error.message);
      throw error;
    }
  };
})();
