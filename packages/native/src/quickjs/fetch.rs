use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::future::AbortHandle;
use gpui::http_client::{AsyncBody, HttpRequestExt, RedirectPolicy, Request, Url};
use rquickjs::{ArrayBuffer, Ctx, Exception, Function, Object, Persistent};
use serde::Deserialize;

use super::{QuickJsRuntime, Shared};
use crate::script::http::{FetchCompletion, FetchRequest, MAX_BODY};

#[derive(Default)]
pub(super) struct FetchState {
    pending: HashMap<u64, Pending>,
    queued: Vec<FetchRequest>,
}

struct Pending {
    resolve: Persistent<Function<'static>>,
    reject: Persistent<Function<'static>>,
    abort: AbortHandle,
}

impl Drop for FetchState {
    fn drop(&mut self) {
        for pending in self.pending.values() {
            pending.abort.abort();
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Options {
    method: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
    timeout_ms: u64,
}

pub(super) fn install<'js>(
    ctx: &Ctx<'js>,
    host: &Object<'js>,
    shared: &Rc<Shared>,
) -> rquickjs::Result<()> {
    let state = shared.clone();
    host.set(
        "startFetch",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  url: String,
                  options: String,
                  resolve: Function<'js>,
                  reject: Function<'js>| {
                let build = || -> Result<_, String> {
                    let options: Options =
                        serde_json::from_str(&options).map_err(|error| error.to_string())?;
                    let url = Url::parse(&url).map_err(|error| error.to_string())?;
                    if !matches!(url.scheme(), "http" | "https")
                        || url.host_str().is_none()
                        || !url.username().is_empty()
                        || url.password().is_some()
                    {
                        return Err(
                            "fetch requires an absolute HTTP(S) URL without credentials".into()
                        );
                    }
                    if options.timeout_ms == 0 || options.timeout_ms > 300_000 {
                        return Err("timeoutMs must be between 1 and 300000".into());
                    }
                    let method = options.method.to_ascii_uppercase();
                    if options.body.is_some() && matches!(method.as_str(), "GET" | "HEAD") {
                        return Err("GET/HEAD requests cannot have a body".into());
                    }
                    let body = options.body.unwrap_or_default().into_bytes();
                    if body.len() > MAX_BODY {
                        return Err("fetch body exceeds 8 MiB".into());
                    }
                    let mut builder = Request::builder()
                        .uri(url.as_str())
                        .method(method.as_str())
                        .timeout(Duration::from_millis(options.timeout_ms))
                        .follow_redirects(RedirectPolicy::NoFollow);
                    for (name, value) in options.headers {
                        builder = builder.header(name, value);
                    }
                    builder
                        .body(AsyncBody::from(body))
                        .map_err(|error| error.to_string())
                };
                let request = build().map_err(|error| Exception::throw_type(&ctx, &error))?;
                let mut fetch = state.fetch.borrow_mut();
                if fetch.pending.len() >= 64 {
                    return Err(Exception::throw_type(
                        &ctx,
                        "at most 64 fetch requests may be pending",
                    ));
                }
                static NEXT: AtomicU64 = AtomicU64::new(1);
                let id = NEXT.fetch_add(1, Ordering::Relaxed);
                let (abort, registration) = AbortHandle::new_pair();
                fetch.pending.insert(
                    id,
                    Pending {
                        resolve: Persistent::save(&ctx, resolve),
                        reject: Persistent::save(&ctx, reject),
                        abort,
                    },
                );
                fetch.queued.push(FetchRequest {
                    id,
                    request,
                    registration,
                });
                Ok(id)
            },
        )?,
    )?;
    let state = shared.clone();
    host.set(
        "cancelFetch",
        Function::new(ctx.clone(), move |id: u64| {
            let mut fetch = state.fetch.borrow_mut();
            if let Some(pending) = fetch.pending.remove(&id) {
                pending.abort.abort();
            }
            fetch.queued.retain(|request| request.id != id);
        })?,
    )?;
    Ok(())
}

impl QuickJsRuntime {
    pub(crate) fn take_fetch_requests(&self) -> Vec<FetchRequest> {
        std::mem::take(&mut self.shared.fetch.borrow_mut().queued)
    }

    pub(crate) fn settle_fetch(&self, completion: FetchCompletion) -> Result<(), String> {
        let pending = self
            .shared
            .fetch
            .borrow_mut()
            .pending
            .remove(&completion.id);
        let Some(pending) = pending else {
            return Ok(());
        };
        self.call(|ctx, _| match completion.result {
            Err(error) => pending.reject.restore(ctx)?.call::<_, ()>((error,)),
            Ok(response) => {
                let value = Object::new(ctx.clone())?;
                value.set("status", response.status)?;
                let mut headers: Vec<(String, String)> = Vec::new();
                for (name, bytes) in response.headers {
                    headers.push((name, String::from_utf8_lossy(&bytes).into_owned()));
                }
                value.set("headers", serde_json::to_string(&headers).unwrap())?;
                value.set("text", String::from_utf8_lossy(&response.body).into_owned())?;
                value.set("bytes", ArrayBuffer::new(ctx.clone(), response.body)?)?;
                pending.resolve.restore(ctx)?.call::<_, ()>((value,))
            }
        })
    }
}
