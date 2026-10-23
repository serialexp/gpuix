use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use futures::future::{AbortHandle, AbortRegistration, Abortable};
use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, HttpRequestExt, RedirectPolicy, Request, Url};
use mlua::{AnyUserData, Function, Lua, LuaSerdeExt, Table, UserData, Value};

const MAX_PENDING: usize = 64;
const MAX_BODY: usize = 8 * 1024 * 1024;

pub(crate) struct FetchRequest {
    id: u64,
    request: Request<AsyncBody>,
    registration: AbortRegistration,
}

pub(crate) struct FetchResponse {
    status: u16,
    headers: Vec<(String, Vec<u8>)>,
    body: Vec<u8>,
}

pub(crate) struct FetchCompletion {
    id: u64,
    result: Result<FetchResponse, String>,
}

struct Pending {
    callback: Function,
    abort: AbortHandle,
}

#[derive(Default)]
pub(crate) struct LuaFetch {
    next_id: u64,
    pending: HashMap<u64, Pending>,
    queued: Vec<FetchRequest>,
    dispatch: Option<Box<dyn Fn(FetchRequest) + Send>>,
}

impl Drop for LuaFetch {
    fn drop(&mut self) {
        for pending in self.pending.values() {
            pending.abort.abort();
        }
    }
}

impl LuaFetch {
    pub(crate) fn set_dispatcher(&mut self, dispatch: impl Fn(FetchRequest) + Send + 'static) {
        for request in self.queued.drain(..) {
            dispatch(request);
        }
        self.dispatch = Some(Box::new(dispatch));
    }

    fn cancel(&mut self, id: u64) -> bool {
        let Some(pending) = self.pending.remove(&id) else {
            return false;
        };
        pending.abort.abort();
        self.queued.retain(|request| request.id != id);
        true
    }

    pub(crate) fn take_callback(&mut self, id: u64) -> Option<Function> {
        self.pending.remove(&id).map(|pending| pending.callback)
    }
}

struct FetchHandle {
    id: u64,
    owner: Weak<Mutex<LuaFetch>>,
}
impl UserData for FetchHandle {}

pub(crate) async fn execute(
    request: FetchRequest,
    client: Arc<dyn HttpClient>,
) -> Option<FetchCompletion> {
    let id = request.id;
    let result = Abortable::new(
        async move {
            let mut response = client
                .send(request.request)
                .await
                .map_err(|error| error.to_string())?;
            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .map(|(name, value)| (name.to_string(), value.as_bytes().to_vec()))
                .collect();
            let mut body = Vec::new();
            response
                .body_mut()
                .take((MAX_BODY + 1) as u64)
                .read_to_end(&mut body)
                .await
                .map_err(|error| error.to_string())?;
            if body.len() > MAX_BODY {
                return Err("fetch response exceeds 8 MiB".into());
            }
            Ok(FetchResponse {
                status,
                headers,
                body,
            })
        },
        request.registration,
    )
    .await
    .ok()?;
    Some(FetchCompletion { id, result })
}

pub(crate) fn deliver(
    lua: &Lua,
    callback: Function,
    completion: FetchCompletion,
) -> mlua::Result<()> {
    match completion.result {
        Err(error) => callback.call::<()>((Value::Nil, error)),
        Ok(response) => {
            let value = lua.create_table()?;
            value.set("status", response.status)?;
            value.set("ok", (200..300).contains(&response.status))?;
            value.set("body", lua.create_string(&response.body)?)?;
            let headers = lua.create_table()?;
            for (name, bytes) in response.headers {
                let bytes =
                    if let Some(previous) = headers.get::<Option<mlua::String>>(name.as_str())? {
                        let mut joined = previous.as_bytes().to_vec();
                        joined.extend_from_slice(b", ");
                        joined.extend_from_slice(&bytes);
                        joined
                    } else {
                        bytes
                    };
                headers.set(name, lua.create_string(&bytes)?)?;
            }
            value.set("headers", headers)?;
            value.set(
                "json",
                lua.create_function(|lua, response: Table| {
                    let body: mlua::String = response.get("body")?;
                    let value: serde_json::Value =
                        serde_json::from_slice(&body.as_bytes()).map_err(mlua::Error::external)?;
                    lua.to_value(&value)
                })?,
            )?;
            callback.call::<()>((value, Value::Nil))
        }
    }
}

pub(crate) fn completion_id(completion: &FetchCompletion) -> u64 {
    completion.id
}

pub(crate) fn install(lua: &Lua, api: &Table, state: &Arc<Mutex<LuaFetch>>) -> mlua::Result<()> {
    let weak = Arc::downgrade(state);
    api.set(
        "fetch",
        lua.create_function(
            move |lua, (url, options, callback): (String, Value, Option<Function>)| {
                let (options, callback) = match (options, callback) {
                    (Value::Function(callback), None) => (None, callback),
                    (Value::Nil, Some(callback)) => (None, callback),
                    (Value::Table(options), Some(callback)) => (Some(options), callback),
                    _ => {
                        return Err(mlua::Error::runtime(
                            "fetch expects (url, callback) or (url, options, callback)",
                        ))
                    }
                };
                let url = Url::parse(&url).map_err(mlua::Error::external)?;
                if !matches!(url.scheme(), "http" | "https")
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err(mlua::Error::runtime(
                        "fetch requires an absolute HTTP(S) URL without embedded credentials",
                    ));
                }
                let mut method = "GET".to_string();
                let mut body = Vec::new();
                let mut timeout = 30_000;
                let mut headers = None;
                if let Some(options) = options {
                    for key in options.clone().pairs::<String, Value>() {
                        if !matches!(key?.0.as_str(), "method" | "headers" | "body" | "timeoutMs") {
                            return Err(mlua::Error::runtime("unknown fetch option"));
                        }
                    }
                    method = options
                        .get::<Option<String>>("method")?
                        .unwrap_or(method)
                        .to_ascii_uppercase();
                    if let Some(value) = options.get::<Option<mlua::String>>("body")? {
                        body = value.as_bytes().to_vec();
                    }
                    timeout = options.get::<Option<u64>>("timeoutMs")?.unwrap_or(timeout);
                    headers = options.get::<Option<Table>>("headers")?;
                }
                if timeout == 0 || timeout > 300_000 {
                    return Err(mlua::Error::runtime(
                        "timeoutMs must be between 1 and 300000",
                    ));
                }
                if body.len() > MAX_BODY
                    || (!body.is_empty() && matches!(method.as_str(), "GET" | "HEAD"))
                {
                    return Err(mlua::Error::runtime(
                        "fetch body exceeds 8 MiB or is not allowed for GET/HEAD",
                    ));
                }
                let mut builder = Request::builder()
                    .uri(url.as_str())
                    .method(method.as_str())
                    .timeout(Duration::from_millis(timeout))
                    .follow_redirects(RedirectPolicy::NoFollow);
                if let Some(headers) = headers {
                    for header in headers.pairs::<String, String>() {
                        let (name, value) = header?;
                        builder = builder.header(name, value);
                    }
                }
                let request = builder
                    .body(AsyncBody::from(body))
                    .map_err(mlua::Error::external)?;
                let state = weak
                    .upgrade()
                    .ok_or_else(|| mlua::Error::runtime("application has stopped"))?;
                let mut state = state.lock().unwrap();
                if state.pending.len() >= MAX_PENDING {
                    return Err(mlua::Error::runtime(
                        "at most 64 fetch requests may be pending",
                    ));
                }
                state.next_id += 1;
                let id = state.next_id;
                let (abort, registration) = AbortHandle::new_pair();
                state.pending.insert(id, Pending { callback, abort });
                let request = FetchRequest {
                    id,
                    request,
                    registration,
                };
                if let Some(dispatch) = &state.dispatch {
                    dispatch(request);
                } else {
                    state.queued.push(request);
                }
                lua.create_userdata(FetchHandle {
                    id,
                    owner: weak.clone(),
                })
            },
        )?,
    )?;
    api.set(
        "cancel_fetch",
        lua.create_function(|_, handle: AnyUserData| {
            let handle = handle.borrow::<FetchHandle>()?;
            Ok(handle
                .owner
                .upgrade()
                .is_some_and(|owner| owner.lock().unwrap().cancel(handle.id)))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Lua, Arc<Mutex<LuaFetch>>) {
        let lua = Lua::new();
        let state = Arc::new(Mutex::new(LuaFetch::default()));
        let api = lua.create_table().unwrap();
        install(&lua, &api, &state).unwrap();
        lua.globals().set("gpuix", api).unwrap();
        (lua, state)
    }

    #[test]
    fn fetch_preserves_binary_bodies_and_delivers_http_errors_as_responses() {
        let (lua, state) = setup();
        lua.load(
            r#"
            request = gpuix.fetch("http://localhost/api", {
                method = "post", headers = { ["content-type"] = "application/octet-stream" },
                body = "a\0b", timeoutMs = 500,
            }, function(response, error)
                assert(error == nil)
                assert(response.status == 422 and not response.ok)
                assert(response.headers["x-test"] == "yes")
                assert(response.json(response).message == "invalid")
                delivered = true
            end)
        "#,
        )
        .exec()
        .unwrap();
        let request = state.lock().unwrap().queued.pop().unwrap();
        let client = gpui::http_client::FakeHttpClient::create(|mut request| async move {
            assert_eq!(request.method(), "POST");
            assert_eq!(
                request.headers()["content-type"],
                "application/octet-stream"
            );
            let mut body = Vec::new();
            request.body_mut().read_to_end(&mut body).await.unwrap();
            assert_eq!(body, b"a\0b");
            Ok(gpui::http_client::Response::builder()
                .status(422)
                .header("x-test", "yes")
                .body(AsyncBody::from(r#"{"message":"invalid"}"#))
                .unwrap())
        });
        let completion = futures::executor::block_on(execute(request, client)).unwrap();
        let callback = state.lock().unwrap().take_callback(completion.id).unwrap();
        deliver(&lua, callback, completion).unwrap();
        assert!(lua.globals().get::<bool>("delivered").unwrap());
        assert!(state.lock().unwrap().pending.is_empty());
        assert!(!lua
            .load("return gpuix.cancel_fetch(request)")
            .eval::<bool>()
            .unwrap());
    }

    #[test]
    fn cancellation_releases_callbacks_and_suppresses_inflight_completions() {
        let (lua, state) = setup();
        lua.load(
            r#"request = gpuix.fetch("http://localhost/", function() error("must not run") end)"#,
        )
        .exec()
        .unwrap();
        let request = state.lock().unwrap().queued.pop().unwrap();
        assert!(lua
            .load("return gpuix.cancel_fetch(request)")
            .eval::<bool>()
            .unwrap());
        assert!(!lua
            .load("return gpuix.cancel_fetch(request)")
            .eval::<bool>()
            .unwrap());
        assert!(state.lock().unwrap().pending.is_empty());
        let client = gpui::http_client::FakeHttpClient::create(|_| async {
            panic!("cancelled request sent")
        });
        assert!(futures::executor::block_on(execute(request, client)).is_none());
    }

    #[test]
    fn validation_rejects_invalid_requests_before_retaining_callbacks() {
        let (lua, state) = setup();
        for arguments in [
            r#""file:///etc/passwd""#,
            r#""http://user:secret@localhost/""#,
            r#""/relative""#,
            r#""http://localhost/", {timeoutMs = 0}"#,
            r#""http://localhost/", {timeoutMs = 300001}"#,
            r#""http://localhost/", {body = "no"}"#,
            r#""http://localhost/", {headers = {bad = "a\r\nb"}}"#,
            r#""http://localhost/", {unknown = true}"#,
        ] {
            assert!(
                lua.load(format!("gpuix.fetch({arguments}, function() end)"))
                    .exec()
                    .is_err(),
                "{arguments}"
            );
        }
        assert!(state.lock().unwrap().pending.is_empty());
        assert!(state.lock().unwrap().queued.is_empty());
    }

    #[test]
    fn queued_requests_are_bounded_and_cancellation_frees_a_slot() {
        let (lua, state) = setup();
        lua.load(
            r#"
            handles = {}
            for index = 1, 64 do
                handles[index] = gpuix.fetch("http://localhost/", function() end)
            end
            assert(not pcall(gpuix.fetch, "http://localhost/", function() end))
            assert(gpuix.cancel_fetch(handles[1]))
            gpuix.fetch("http://localhost/", function() end)
        "#,
        )
        .exec()
        .unwrap();
        assert_eq!(state.lock().unwrap().pending.len(), MAX_PENDING);
        assert_eq!(state.lock().unwrap().queued.len(), MAX_PENDING);
    }

    #[test]
    fn transport_errors_and_oversized_responses_use_the_error_callback() {
        for oversized in [false, true] {
            let (lua, state) = setup();
            lua.load(
                r#"gpuix.fetch("http://localhost/", function(response, error)
                assert(response == nil and type(error) == "string")
                message = error
            end)"#,
            )
            .exec()
            .unwrap();
            let request = state.lock().unwrap().queued.pop().unwrap();
            let client = gpui::http_client::FakeHttpClient::create(move |_| async move {
                if !oversized {
                    return Err(gpui::http_client::anyhow!("offline"));
                }
                Ok(gpui::http_client::Response::builder()
                    .body(AsyncBody::from(vec![0; MAX_BODY + 1]))
                    .unwrap())
            });
            let completion = futures::executor::block_on(execute(request, client)).unwrap();
            let callback = state.lock().unwrap().take_callback(completion.id).unwrap();
            deliver(&lua, callback, completion).unwrap();
            let message: String = lua.globals().get("message").unwrap();
            assert!(message.contains(if oversized { "8 MiB" } else { "offline" }));
        }
    }
}
