(() => {
  let states = new Map();
  let previous = new Map();
  let snapshots = new Map();
  return {
    provide(path) {
      const data = states.get(path)?.data ?? {};
      const state = { data, accepts: [], disposes: [], declined: false, invalidated: false };
      states.set(path, state);
      return {
        data,
        accept(callback) {
          if (callback !== undefined && typeof callback !== "function") {
            throw new TypeError("Only self-accepting hot modules are supported");
          }
          state.accepts.push(callback);
        },
        dispose(callback) { state.disposes.push(callback); },
        decline() { state.declined = true; },
        invalidate() { state.invalidated = true; }
      };
    },
    accepts(path) {
      const state = states.get(path);
      return !!state && state.accepts.length > 0 && !state.declined && !state.invalidated;
    },
    begin(paths) {
      previous = new Map(states);
      snapshots = new Map([...states].map(([path, state]) => [path, { ...state.data }]));
      for (const path of paths) {
        const state = states.get(path);
        if (state) for (const dispose of state.disposes) dispose(state.data);
      }
    },
    commit(path, namespace) {
      const old = previous.get(path);
      if (old) for (const accept of old.accepts) accept?.(namespace);
      const state = states.get(path);
      return !!state && state.accepts.length > 0 && !state.declined && !state.invalidated && !old?.invalidated;
    },
    finish() { previous.clear(); snapshots.clear(); },
    rollback() {
      for (const [path, state] of previous) {
        for (const key of Object.keys(state.data)) delete state.data[key];
        Object.assign(state.data, snapshots.get(path));
      }
      if (previous.size) states = previous;
      previous = new Map();
      snapshots.clear();
    }
  };
})()
