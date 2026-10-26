use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use serde_json::{json, Value};

pub(crate) struct ResetRequest(pub std::sync::mpsc::SyncSender<Value>);

impl ResetRequest {
    pub fn finish(self, result: Result<Value, String>) {
        let value = match result {
            Ok(value) => json!({"result": value}),
            Err(error) => json!({"error": error}),
        };
        let _ = self.0.send(value);
    }
}

struct Control(UnboundedSender<ResetRequest>);
impl gpui::Global for Control {}

pub(crate) fn install(cx: &mut gpui::App) -> UnboundedReceiver<ResetRequest> {
    let (sender, receiver) = unbounded();
    cx.set_global(Control(sender));
    receiver
}

pub(crate) fn request(response: std::sync::mpsc::SyncSender<Value>, cx: &gpui::App) {
    let request = ResetRequest(response);
    if let Some(control) = cx.try_global::<Control>() {
        if let Err(error) = control.0.unbounded_send(request) {
            error
                .into_inner()
                .finish(Err("application reset handler stopped".into()));
        }
    } else {
        request.finish(Err("application reset is unavailable".into()));
    }
}

pub(crate) fn paint(cx: &mut gpui::App) -> Result<Value, String> {
    let mut windows = Vec::new();
    for handle in cx.windows() {
        if handle.downcast::<crate::renderer::GpuixView>().is_none() {
            continue;
        }
        handle
            .update(cx, |_, window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            })
            .map_err(|error| error.to_string())?;
        windows.push(handle.window_id().as_u64());
    }
    Ok(json!({"windows": windows}))
}
