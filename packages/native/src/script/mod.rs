//! What the embedded script engines share: the application and window
//! registry, and the source watcher for hot reload. Each engine keeps its own
//! values (render callbacks, handlers) and its own error wording.

pub(crate) mod app;
pub(crate) mod reload;
