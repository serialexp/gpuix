(() => {
  "use strict";
  const { startFetch, cancelFetch } = globalThis.__gpuix;
  const abortError = () => Object.assign(new Error("The operation was aborted"), { name: "AbortError" });

  class AbortSignal {
    #listeners = new Set();
    #aborted = false;
    #reason;
    get aborted() { return this.#aborted; }
    get reason() { return this.#reason; }
    onabort = null;
    addEventListener(type, listener) {
      if (type === "abort" && listener) this.#listeners.add(listener);
    }
    removeEventListener(type, listener) {
      if (type === "abort") this.#listeners.delete(listener);
    }
    throwIfAborted() { if (this.#aborted) throw this.#reason; }
    static abort(reason = abortError()) {
      const controller = new AbortController();
      controller.abort(reason);
      return controller.signal;
    }
    static _abort(signal, reason) {
      if (signal.#aborted) return;
      signal.#aborted = true;
      signal.#reason = reason;
      const event = { type: "abort", target: signal };
      for (const listener of [...signal.#listeners, signal.onabort]) {
        if (!listener) continue;
        try {
          if (typeof listener === "function") listener.call(signal, event);
          else listener.handleEvent(event);
        } catch (error) { globalThis.reportError(error); }
      }
      signal.#listeners.clear();
    }
  }
  class AbortController {
    #signal = new AbortSignal();
    get signal() { return this.#signal; }
    abort(reason = abortError()) { AbortSignal._abort(this.#signal, reason); }
  }

  class Headers {
    #values = new Map();
    constructor(init = []) {
      const entries = init != null && typeof init[Symbol.iterator] === "function"
        ? init : Object.entries(init ?? {});
      for (const [name, value] of entries) this.append(name, value);
    }
    append(name, value) {
      name = String(name).toLowerCase();
      value = String(value).trim();
      this.#values.set(name, this.has(name) ? this.get(name) + ", " + value : value);
    }
    set(name, value) { this.#values.set(String(name).toLowerCase(), String(value).trim()); }
    get(name) { return this.#values.get(String(name).toLowerCase()) ?? null; }
    has(name) { return this.#values.has(String(name).toLowerCase()); }
    delete(name) { this.#values.delete(String(name).toLowerCase()); }
    entries() { return this.#values.entries(); }
    keys() { return this.#values.keys(); }
    values() { return this.#values.values(); }
    [Symbol.iterator]() { return this.entries(); }
    forEach(callback, receiver) {
      for (const [name, value] of this) callback.call(receiver, value, name, this);
    }
  }

  globalThis.AbortController = AbortController;
  globalThis.AbortSignal = AbortSignal;
  globalThis.Headers = Headers;
  globalThis.fetch = (url, options = {}) => new Promise((resolve, reject) => {
    if (typeof url !== "string") throw new TypeError("fetch expects an absolute URL string");
    for (const key of Object.keys(options)) {
      if (!["method", "headers", "body", "signal", "timeoutMs"].includes(key)) {
        throw new TypeError("Unsupported fetch option: " + key);
      }
    }
    const signal = options.signal;
    if (signal != null && !(signal instanceof AbortSignal)) throw new TypeError("Invalid AbortSignal");
    if (signal?.aborted) { reject(signal.reason); return; }
    if (options.body != null && typeof options.body !== "string") {
      throw new TypeError("Embedded fetch currently supports string request bodies only");
    }
    const method = String(options.method ?? "GET").toUpperCase();
    const headers = new Headers(options.headers);
    if (options.body != null && !headers.has("content-type")) {
      headers.set("content-type", "text/plain;charset=UTF-8");
    }
    let id;
    const cleanup = () => signal?.removeEventListener("abort", abort);
    const abort = () => {
      cancelFetch(id);
      cleanup();
      reject(signal.reason);
    };
    id = startFetch(url, JSON.stringify({
      method, headers: [...headers], body: options.body ?? null,
      timeoutMs: options.timeoutMs ?? 30000,
    }), (response) => {
      cleanup();
      let used = false;
      let bytes = response.bytes;
      let text = response.text;
      const consume = async (kind) => {
        if (used) throw new TypeError("Response body has already been consumed");
        used = true;
        const value = kind === "arrayBuffer" ? bytes : text;
        bytes = text = undefined;
        return kind === "json" ? JSON.parse(value) : value;
      };
      resolve({
        status: response.status, ok: response.status >= 200 && response.status < 300,
        url, redirected: false, headers: new Headers(JSON.parse(response.headers)),
        get bodyUsed() { return used; },
        text: () => consume("text"), json: () => consume("json"),
        arrayBuffer: () => consume("arrayBuffer"),
      });
    }, (error) => { cleanup(); reject(new TypeError(error)); });
    signal?.addEventListener("abort", abort);
  });
})();
