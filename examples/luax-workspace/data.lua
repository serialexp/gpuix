local conversations = {
    { id = "runtime", title = "Embedded Lua runtime", subtitle = "Reconciliation and hooks" },
    { id = "protocol", title = "Binary protocol", subtitle = "Snapshot boundary costs" },
    { id = "selection", title = "Native text selection", subtitle = "Paint-order registry" },
    { id = "timeline", title = "Timeline interactions", subtitle = "Dragging without overlays" },
    { id = "markdown", title = "Markdown renderer", subtitle = "Native blocks and code" },
}

local activities = {}
for index = 1, 18 do
    activities[index] = {
        id = "activity-" .. index,
        title = index % 3 == 0 and "Reconciled retained subtree" or "Rendered component module",
        detail = "Update " .. index .. " · " .. (index * 7) .. " nodes inspected",
        status = index % 4 == 0 and "cached" or "complete",
    }
end

return {
    conversations = conversations,
    activities = activities,
    overview = [[
# Native Lua workspace

This screen is composed from **source-relative Lua modules**. GPUI events stay
inside the native process, state is retained by Rust hook slots, and LuaX emits
packed host handles directly into the render arena.

- Click a conversation in the sidebar.
- Focus the sidebar and use Up, Down, Home, or End to move its selection.
- Switch among Markdown, highlighted Rust, a native diff, and image widgets.
- Scroll and select activity rows to exercise `virtual-list` and `memo_batch`.
- Edit the composer to exercise native input events.
]],
    code = [[
pub fn dispatch_event(payload: EventPayload) {
    event_sender.unbounded_send(payload)?;
    runtime.dispatch_event(payload, &mut tree)?;
    view.notify();
}
]],
    patch = [[
diff --git a/src/runtime.rs b/src/runtime.rs
--- a/src/runtime.rs
+++ b/src/runtime.rs
@@ -41,8 +41,11 @@ impl Runtime {
     pub fn render(&mut self) -> Result<()> {
-        let snapshot = self.lua.to_value(&self.root)?;
-        self.renderer.replace(snapshot)
+        let handles = self.lua.render_handles()?;
+        let mutations = self.reconciler.diff(handles);
+        self.renderer.apply(mutations)?;
+        self.view.notify();
+        Ok(())
     }
 }
]],
}
