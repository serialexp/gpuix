use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

#[cfg(feature = "test-support")]
use base64::Engine as _;
use futures::{channel::mpsc, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::renderer::GpuixView;

#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "camelCase", deny_unknown_fields)]
enum Command {
    Health {},
    Windows {},
    Tree {
        window: u64,
        #[serde(default)]
        details: bool,
    },
    Subtree {
        window: u64,
        #[serde(default, rename = "nodeId")]
        node_id: Option<u64>,
        #[serde(default, rename = "testId")]
        test_id: Option<String>,
        #[serde(default)]
        details: bool,
        #[serde(default, rename = "ifRevision")]
        if_revision: Option<String>,
    },
    Screenshot {
        window: u64,
    },
    Geometry {
        window: u64,
    },
    Pick {
        window: u64,
        x: f64,
        y: f64,
    },
    Highlight {
        window: u64,
        #[serde(rename = "nodeId")]
        node_id: Option<u64>,
    },
    Click {
        window: u64,
        x: f64,
        y: f64,
        #[serde(default)]
        button: u32,
    },
    Press {
        window: u64,
        keys: String,
    },
}

struct Request {
    command: Command,
    response: std::sync::mpsc::SyncSender<Value>,
    deadline: Instant,
}

enum HttpRequest {
    Inspector,
    Rpc(Command),
}

pub(crate) struct Server {
    requests: mpsc::UnboundedReceiver<Request>,
    stopped: Arc<AtomicBool>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}

impl Server {
    pub(crate) fn start(port: Option<u16>) -> Result<Option<Self>, String> {
        let Some(port) = port else { return Ok(None) };
        let token = std::env::var("GPUIX_DEV_TOKEN")
            .ok()
            .filter(|token| {
                token.len() >= 32 && token.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
            .ok_or("GPUIX_DEV_TOKEN must contain at least 32 ASCII letters or digits")?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .map_err(|error| format!("dev server: {error}"))?;
        Self::listen(listener, token).map(Some)
    }

    fn listen(listener: TcpListener, token: String) -> Result<Self, String> {
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let origin = format!(
            "http://{}",
            listener.local_addr().map_err(|error| error.to_string())?
        );
        eprintln!("gpx: inspector {origin}/inspector");
        eprintln!(
            "gpx: dev API http://{}/rpc",
            listener.local_addr().map_err(|error| error.to_string())?
        );
        let (sender, requests) = mpsc::unbounded();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if stream.set_nonblocking(false).is_err() {
                            continue;
                        }
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                        let incoming = read_http_request(&mut stream, &token, Some(&origin));
                        if matches!(incoming, Ok(HttpRequest::Inspector)) {
                            let body = include_str!("inspector.html");
                            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len());
                            continue;
                        }
                        let result = incoming.and_then(|incoming| {
                            let HttpRequest::Rpc(command) = incoming else {
                                unreachable!()
                            };
                            let (response, receiver) = std::sync::mpsc::sync_channel(1);
                            sender
                                .unbounded_send(Request {
                                    command,
                                    response,
                                    deadline: Instant::now() + Duration::from_secs(5),
                                })
                                .map_err(|_| "app closed".to_string())?;
                            receiver
                                .recv_timeout(Duration::from_secs(5))
                                .map_err(|_| "app did not respond within five seconds".to_string())
                        });
                        let (status, body) = match result {
                            Ok(body) => ("200 OK", body),
                            Err(error) => ("400 Bad Request", json!({"error": error})),
                        };
                        let body = body.to_string();
                        let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self { requests, stopped })
    }

    pub(crate) fn attach(mut self, cx: &mut gpui::App) {
        crate::automation::enable_inspector();
        cx.spawn(async move |cx| {
            while let Some(request) = self.requests.next().await {
                if Instant::now() >= request.deadline {
                    continue;
                }
                cx.update(|cx| dispatch(request, cx));
            }
        })
        .detach();
    }
}

fn respond(response: std::sync::mpsc::SyncSender<Value>, result: Result<Value, String>) {
    let body = match result {
        Ok(value) => json!({"result": value}),
        Err(error) => json!({"error": error}),
    };
    let _ = response.send(body);
}

fn dispatch(request: Request, cx: &mut gpui::App) {
    if let Command::Screenshot { window: id } = request.command {
        #[cfg(feature = "test-support")]
        {
            let handle = cx.windows().into_iter().find(|window| {
                window.window_id().as_u64() == id && window.downcast::<GpuixView>().is_some()
            });
            let Some(handle) = handle else {
                respond(request.response, Err("window not found".into()));
                return;
            };
            let result = handle.update(cx, move |_, window, cx| {
                window.refresh();
                let arena = window.draw(cx);
                let image = window.render_to_image().map_err(|error| error.to_string());
                arena.clear(cx);
                image.and_then(|image| encode_screenshot(image, window.scale_factor()))
            });
            respond(
                request.response,
                result
                    .map_err(|error| error.to_string())
                    .and_then(|value| value),
            );
        }
        #[cfg(not(feature = "test-support"))]
        {
            let _ = id;
            respond(
                request.response,
                Err("screenshots require a build with test-support".into()),
            );
        }
    } else {
        respond(request.response, execute(request.command, cx));
    }
}

#[cfg(feature = "test-support")]
fn encode_screenshot(image: image::RgbaImage, scale_factor: f32) -> Result<Value, String> {
    let (width, height) = image.dimensions();
    if u64::from(width) * u64::from(height) > 16_777_216 {
        return Err("screenshot exceeds 16 megapixels".into());
    }
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(
        json!({"mimeType": "image/png", "encoding": "base64", "data": base64::engine::general_purpose::STANDARD.encode(bytes.into_inner()), "width": width, "height": height, "scaleFactor": scale_factor}),
    )
}

#[cfg(test)]
fn read_request(stream: &mut impl Read, token: &str) -> Result<Command, String> {
    match read_http_request(stream, token, None)? {
        HttpRequest::Rpc(command) => Ok(command),
        HttpRequest::Inspector => Err("expected RPC".into()),
    }
}

fn read_http_request(
    stream: &mut impl Read,
    token: &str,
    origin: Option<&str>,
) -> Result<HttpRequest, String> {
    let mut reader = BufReader::new(stream.take(16384));
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    let inspector = line == "GET /inspector HTTP/1.1\r\n";
    if !inspector && line != "POST /rpc HTTP/1.1\r\n" {
        return Err("use POST /rpc or GET /inspector HTTP/1.1".into());
    }
    let mut authorized = false;
    let mut length = None;
    let mut host_seen = false;
    let mut origin_seen = false;
    loop {
        line.clear();
        if reader
            .read_line(&mut line)
            .map_err(|error| error.to_string())?
            == 0
        {
            return Err("incomplete headers".into());
        }
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.split_once(':').ok_or("invalid header")?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err("transfer encoding is not allowed".into());
        }
        if name.eq_ignore_ascii_case("host") {
            if host_seen
                || origin.is_some_and(|origin| origin.strip_prefix("http://") != Some(value))
            {
                return Err("invalid host; use the printed 127.0.0.1 address".into());
            }
            host_seen = true;
        }
        if name.eq_ignore_ascii_case("origin") {
            if origin_seen || origin != Some(value) {
                return Err("foreign browser origins are not allowed".into());
            }
            origin_seen = true;
        }
        if name.eq_ignore_ascii_case("authorization") {
            authorized = value == format!("Bearer {token}");
        }
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err("duplicate content length".into());
            }
            length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| "invalid content length")?,
            );
        }
    }
    if origin.is_some() && !host_seen {
        return Err("host required".into());
    }
    if inspector {
        return Ok(HttpRequest::Inspector);
    }
    if !authorized {
        return Err("invalid bearer token".into());
    }
    let length = length
        .filter(|length| *length <= 8192)
        .ok_or("content length required, maximum 8192 bytes")?;
    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .map_err(|error| error.to_string())?;
    serde_json::from_slice(&body)
        .map(HttpRequest::Rpc)
        .map_err(|error| error.to_string())
}

fn execute(command: Command, cx: &mut gpui::App) -> Result<Value, String> {
    let windows = cx
        .windows()
        .into_iter()
        .filter_map(|window| window.downcast::<GpuixView>())
        .collect::<Vec<_>>();
    let target = match &command {
        Command::Health {} => {
            return Ok(json!({"ready": !windows.is_empty(), "pid": std::process::id()}))
        }
        Command::Windows {} => {
            return Ok(json!(windows
                .iter()
                .map(|window| window.window_id().as_u64())
                .collect::<Vec<_>>()))
        }
        Command::Tree { window, .. }
        | Command::Subtree { window, .. }
        | Command::Screenshot { window }
        | Command::Geometry { window }
        | Command::Pick { window, .. }
        | Command::Highlight { window, .. }
        | Command::Click { window, .. }
        | Command::Press { window, .. } => *window,
    };
    let handle = windows
        .into_iter()
        .find(|window| window.window_id().as_u64() == target)
        .ok_or("window not found")?;
    if let Command::Geometry { .. } = command {
        let bounds = crate::automation::window_bounds(handle.window_id());
        return Ok(json!(bounds
            .into_iter()
            .map(|(id, rect)| (
                id.to_string(),
                json!({"x":rect.x,"y":rect.y,"width":rect.width,"height":rect.height})
            ))
            .collect::<serde_json::Map<_, _>>()));
    }
    if let Command::Pick { x, y, .. } = command {
        if !x.is_finite() || !y.is_finite() {
            return Err("coordinates must be finite".into());
        }
        let candidates = crate::automation::painted_candidates(handle.window_id(), x, y);
        return handle
            .read_with(cx, |view, _| {
                let tree = view.tree.lock().unwrap();
                let node = candidates.into_iter().find(|id| mounted_node(&tree, *id));
                json!({"nodeId": node})
            })
            .map_err(|error| error.to_string());
    }
    if let Command::Highlight { node_id, .. } = command {
        if let Some(id) = node_id {
            let mounted = handle
                .read_with(cx, |view, _| mounted_node(&view.tree.lock().unwrap(), id))
                .map_err(|error| error.to_string())?;
            if !mounted {
                return Err("node not found in the mounted tree".into());
            }
        }
        crate::automation::highlight(handle.window_id(), node_id);
        handle
            .update(cx, |_, window, _| window.refresh())
            .map_err(|error| error.to_string())?;
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(2050))
                .await;
            let _ = handle.update(cx, |_, window, _| window.refresh());
        })
        .detach();
        return Ok(json!({"highlighted": node_id}));
    }
    if let Command::Subtree {
        node_id,
        test_id,
        details,
        if_revision,
        ..
    } = command
    {
        return handle
            .read_with(cx, |view, _| {
                subtree(
                    &view.tree.lock().unwrap(),
                    node_id,
                    test_id.as_deref(),
                    details,
                    if_revision.as_deref(),
                )
            })
            .map_err(|error| error.to_string())?;
    }
    if let Command::Tree { details, .. } = command {
        return handle
            .read_with(cx, |view, _| {
                let tree = view.tree.lock().unwrap();
                if details {
                    tree.to_json(&HashMap::new())
                } else {
                    tree.to_automation_json(&HashMap::new())
                }
            })
            .map_err(|error| error.to_string());
    }
    gpui::AnyWindowHandle::from(handle)
        .update(cx, |_, window, cx| {
            match command {
                Command::Click { x, y, button, .. } => {
                    if !x.is_finite() || !y.is_finite() {
                        return Err("coordinates must be finite".into());
                    }
                    if button > 2 {
                        return Err("button must be 0 (left), 1 (middle), or 2 (right)".into());
                    }
                    crate::automation::dispatch_click(
                        window,
                        cx,
                        x,
                        y,
                        button,
                        gpui::Modifiers::default(),
                    );
                }
                Command::Press { keys, .. } => {
                    crate::automation::dispatch_keystrokes(window, cx, &keys)?
                }
                _ => unreachable!(),
            }
            Ok(json!({"dispatched": true}))
        })
        .map_err(|error| error.to_string())?
}

fn mounted_node(tree: &crate::retained_tree::RetainedTree, id: u64) -> bool {
    let Some(root) = tree.root_id else {
        return false;
    };
    let mut current = Some(id);
    while let Some(id) = current {
        let Some(node) = tree.elements.get(&id) else {
            return false;
        };
        if id == root {
            return true;
        }
        current = node.parent;
    }
    false
}

fn subtree(
    tree: &crate::retained_tree::RetainedTree,
    node_id: Option<u64>,
    test_id: Option<&str>,
    details: bool,
    if_revision: Option<&str>,
) -> Result<Value, String> {
    if node_id.is_some() && test_id.is_some() {
        return Err("provide nodeId or testId, not both".into());
    }
    let root = tree.root_id.ok_or("window has no mounted tree")?;
    let id = if let Some(id) = node_id {
        if !mounted_node(tree, id) {
            return Err("node not found in the mounted tree".into());
        }
        id
    } else if let Some(test_id) = test_id {
        let mut pending = vec![root];
        let mut found = None;
        while let Some(id) = pending.pop() {
            let Some(node) = tree.elements.get(&id) else {
                continue;
            };
            if node.test_id.as_deref() == Some(test_id) {
                if found.replace(id).is_some() {
                    return Err("testId matches multiple nodes".into());
                }
            }
            pending.extend(node.children.iter().copied());
        }
        found.ok_or("testId not found in the mounted tree")?
    } else {
        root
    };
    let node = tree.elements.get(&id).ok_or("node not found")?;
    static SESSION: std::sync::LazyLock<u128> = std::sync::LazyLock::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    });
    let revision = format!(
        "{}-{}-{}-{}-{}-{}",
        std::process::id(),
        *SESSION,
        tree.automation_id,
        id,
        node.subtree_revision,
        details
    );
    let unchanged = if_revision == Some(revision.as_str());
    let mut result = json!({"nodeId": id, "revision": revision, "unchanged": unchanged});
    if !unchanged {
        result["tree"] = tree.subtree_to_json(id, details);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section_tree() -> crate::retained_tree::RetainedTree {
        let mut tree = crate::retained_tree::RetainedTree::new();
        for id in 1..=5 {
            tree.create_element(id, "div".into());
        }
        tree.set_root(1);
        tree.append_child(1, 2);
        tree.append_child(1, 3);
        tree.append_child(2, 4);
        tree.set_custom_prop(2, "testId".into(), json!("section"));
        tree.set_text(4, "inside".into());
        tree.set_text(3, "outside".into());
        tree
    }

    #[test]
    fn subtree_exports_only_the_selection_and_skips_unchanged_data() {
        let mut tree = section_tree();
        let first = subtree(&tree, None, Some("section"), false, None).unwrap();
        assert_eq!(first["nodeId"], 2);
        assert_eq!(first["tree"]["children"][0]["text"], "inside");
        assert!(!first.to_string().contains("outside"));
        let revision = first["revision"].as_str().unwrap();
        tree.set_text(3, "outside changed".into());
        let same = subtree(&tree, Some(2), None, false, Some(revision)).unwrap();
        assert_eq!(same["unchanged"], true);
        assert!(same.get("tree").is_none());
        tree.set_text(4, "inside changed".into());
        let changed = subtree(&tree, Some(2), None, false, Some(revision)).unwrap();
        assert_eq!(changed["unchanged"], false);
        assert_ne!(changed["revision"], first["revision"]);
        assert_eq!(changed["tree"]["children"][0]["text"], "inside changed");
    }

    #[test]
    fn subtree_rejects_detached_missing_and_ambiguous_selectors() {
        let mut tree = section_tree();
        assert!(subtree(&tree, Some(5), None, false, None).is_err());
        assert!(subtree(&tree, Some(999), None, false, None).is_err());
        assert!(subtree(&tree, Some(2), Some("section"), false, None).is_err());
        assert!(subtree(&tree, None, Some("missing"), false, None).is_err());
        tree.set_custom_prop(3, "testId".into(), json!("section"));
        assert!(subtree(&tree, None, Some("section"), false, None).is_err());
        tree.destroy_element(2);
        assert!(subtree(&tree, Some(2), None, false, None).is_err());
    }

    #[test]
    fn subtree_tokens_distinguish_replaced_trees_and_detail_modes() {
        let tree = section_tree();
        let first = subtree(&tree, Some(2), None, false, None).unwrap();
        let revision = first["revision"].as_str().unwrap();
        assert_eq!(
            subtree(&section_tree(), Some(2), None, false, Some(revision)).unwrap()["unchanged"],
            false
        );
        assert_eq!(
            subtree(&tree, Some(2), None, true, Some(revision)).unwrap()["unchanged"],
            false
        );
        assert_eq!(
            subtree(&tree, None, None, false, None).unwrap()["nodeId"],
            1
        );
    }

    #[test]
    fn subtree_tokens_track_test_ids_events_props_and_structure() {
        let mut tree = section_tree();
        let mut revision = subtree(&tree, Some(2), None, true, None).unwrap()["revision"]
            .as_str()
            .unwrap()
            .to_owned();
        for change in 0..5 {
            match change {
                0 => tree.set_custom_prop(4, "testId".into(), json!("child")),
                1 => tree.set_event_listener(4, "click".into(), true),
                2 => tree.set_custom_prop(4, "value".into(), json!(7)),
                3 => tree.append_child(2, 5),
                _ => {
                    tree.destroy_element(5);
                }
            }
            let result = subtree(&tree, Some(2), None, true, Some(&revision)).unwrap();
            assert_eq!(result["unchanged"], false, "change {change}");
            revision = result["revision"].as_str().unwrap().to_owned();
        }
        tree.set_event_listener(4, "click".into(), true);
        tree.set_custom_prop(4, "testId".into(), json!("child"));
        assert_eq!(
            subtree(&tree, Some(2), None, true, Some(&revision)).unwrap()["unchanged"],
            true
        );
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn screenshot_response_contains_a_decodable_png() {
        let image = image::RgbaImage::from_pixel(3, 2, image::Rgba([20, 40, 60, 255]));
        let result = encode_screenshot(image, 2.0).unwrap();
        assert_eq!(result["width"], 3);
        assert_eq!(result["height"], 2);
        assert_eq!(result["scaleFactor"], 2.0);
        assert_eq!(result["mimeType"], "image/png");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(result["data"].as_str().unwrap())
            .unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (3, 2));
        assert_eq!(decoded.get_pixel(2, 1).0, [20, 40, 60, 255]);
    }

    #[test]
    fn screenshot_rejects_missing_windows_and_file_paths() {
        let cx = gpui::TestAppContext::single();
        let (response, receiver) = std::sync::mpsc::sync_channel(1);
        cx.update(|cx| {
            dispatch(
                Request {
                    command: Command::Screenshot { window: u64::MAX },
                    response,
                    deadline: Instant::now() + Duration::from_secs(5),
                },
                cx,
            )
        });
        assert!(receiver.try_recv().unwrap()["error"].is_string());
        assert!(read_request(
            &mut request(
                "",
                r#"{"method":"screenshot","window":1,"path":"/tmp/app.png"}"#
            )
            .as_bytes(),
            "secret"
        )
        .is_err());
    }

    #[test]
    fn http_server_forwards_commands_and_returns_json() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let mut server = Server::listen(listener, "secret".into()).unwrap();
        let client = std::thread::spawn(move || {
            let mut stream = std::net::TcpStream::connect(address).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let message = request(&format!("Host: {address}\r\n"), r#"{"method":"health"}"#);
            let (headers, body) = message.split_once("\r\n\r\n").unwrap();
            write!(stream, "{headers}\r\n\r\n").unwrap();
            std::thread::sleep(Duration::from_millis(30));
            stream.write_all(body.as_bytes()).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        let request = loop {
            if let Ok(request) = server.requests.try_recv() {
                break request;
            }
            assert!(
                Instant::now() < deadline,
                "server did not accept the split request"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(matches!(request.command, Command::Health {}));
        request
            .response
            .send(json!({"result": {"ready": true}}))
            .unwrap();
        let response = client.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert_eq!(
            serde_json::from_str::<Value>(response.split_once("\r\n\r\n").unwrap().1).unwrap(),
            json!({"result": {"ready": true}})
        );
        drop(server);
    }

    #[test]
    fn commands_discover_windows_and_route_trees_individually() {
        let cx = gpui::TestAppContext::single();
        cx.update(|cx| {
            assert_eq!(execute(Command::Health {}, cx).unwrap()["ready"], false);
            let mut ids = Vec::new();
            for label in ["first", "second"] {
                let mut tree = crate::retained_tree::RetainedTree::new();
                tree.create_element(1, "text".into());
                tree.set_text(1, label.into());
                tree.set_event_listener(1, "click".into(), true);
                tree.set_root(1);
                let window = crate::renderer::open_main_window(
                    &crate::renderer::WindowOptions {
                        focus: Some(false),
                        ..Default::default()
                    },
                    Arc::new(std::sync::Mutex::new(tree)),
                    None,
                    crate::text::SharedSelection::default(),
                    cx,
                )
                .unwrap();
                let id = window.window_id().as_u64();
                ids.push(id);
                crate::automation::enable_inspector();
                gpui::AnyWindowHandle::from(window)
                    .update(cx, |_, window, cx| {
                        let arena = window.draw(cx);
                        arena.clear(cx);
                    })
                    .unwrap();
                let geometry = execute(Command::Geometry { window: id }, cx).unwrap();
                assert!(geometry["1"]["width"].as_f64().unwrap() > 0.0);
                let x = geometry["1"]["x"].as_f64().unwrap() + 1.0;
                let y = geometry["1"]["y"].as_f64().unwrap() + 1.0;
                assert_eq!(
                    execute(Command::Pick { window: id, x, y }, cx).unwrap()["nodeId"],
                    1
                );
                assert!(execute(
                    Command::Highlight {
                        window: id,
                        node_id: Some(1)
                    },
                    cx
                )
                .is_ok());
                assert!(execute(
                    Command::Highlight {
                        window: id,
                        node_id: Some(999)
                    },
                    cx
                )
                .is_err());
                assert!(execute(
                    Command::Highlight {
                        window: id,
                        node_id: None
                    },
                    cx
                )
                .is_ok());
                assert_eq!(
                    execute(
                        Command::Tree {
                            window: id,
                            details: false
                        },
                        cx
                    )
                    .unwrap()["text"],
                    label
                );
                assert_eq!(
                    execute(
                        Command::Tree {
                            window: id,
                            details: true
                        },
                        cx
                    )
                    .unwrap()["events"],
                    json!(["click"])
                );
            }
            assert_eq!(execute(Command::Health {}, cx).unwrap()["ready"], true);
            let result = execute(Command::Windows {}, cx).unwrap();
            for id in ids {
                assert!(result.as_array().unwrap().contains(&json!(id)));
                assert!(
                    execute(Command::Geometry { window: id }, cx).unwrap()["1"]["width"]
                        .as_f64()
                        .unwrap()
                        > 0.0
                );
            }
            assert!(execute(
                Command::Tree {
                    window: u64::MAX,
                    details: false
                },
                cx
            )
            .is_err());
        });
    }

    fn request(headers: &str, body: &str) -> String {
        format!("POST /rpc HTTP/1.1\r\nAuthorization: Bearer secret\r\nContent-Length: {}\r\n{headers}\r\n{body}", body.len())
    }

    #[test]
    fn inspector_allows_only_its_exact_origin_and_host_without_exposing_token() {
        let origin = "http://127.0.0.1:4318";
        let headers = "Host: 127.0.0.1:4318\r\nOrigin: http://127.0.0.1:4318\r\n";
        assert!(matches!(
            read_http_request(
                &mut request(headers, r#"{"method":"geometry","window":1}"#).as_bytes(),
                "secret",
                Some(origin)
            )
            .unwrap(),
            HttpRequest::Rpc(Command::Geometry { window: 1 })
        ));
        let page = "GET /inspector HTTP/1.1\r\nHost: 127.0.0.1:4318\r\n\r\n";
        assert!(matches!(
            read_http_request(&mut page.as_bytes(), "secret", Some(origin)).unwrap(),
            HttpRequest::Inspector
        ));
        for bad in [
            "Host: evil.example\r\n",
            "Host: 127.0.0.1:4318\r\nOrigin: http://evil.example\r\n",
            "Host: 127.0.0.1:4318\r\nOrigin: null\r\n",
            "Host: 127.0.0.1:4318\r\nHost: 127.0.0.1:4318\r\n",
            "",
        ] {
            assert!(read_http_request(
                &mut request(bad, r#"{"method":"windows"}"#).as_bytes(),
                "secret",
                Some(origin)
            )
            .is_err());
        }
        assert!(read_http_request(
            &mut request(headers, r#"{"method":"windows"}"#).as_bytes(),
            "wrong",
            Some(origin)
        )
        .is_err());
        assert!(read_request(
            &mut request("", r#"{"method":"pick","window":1,"x":20,"y":30}"#).as_bytes(),
            "secret"
        )
        .is_ok());
        assert!(read_request(
            &mut request("", r#"{"method":"highlight","window":1,"nodeId":null}"#).as_bytes(),
            "secret"
        )
        .is_ok());
    }

    #[test]
    fn serves_inspector_html_without_app_data_or_credentials() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = Server::listen(listener, "private-test-token".into()).unwrap();
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(stream, "GET /inspector HTTP/1.1\r\nHost: {address}\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("frame-ancestors 'none'"));
        assert!(response.contains("GPUIX Inspector"));
        assert!(!response.contains("private-test-token"));
        drop(server);
    }

    #[test]
    fn validates_commands_and_authentication() {
        assert!(matches!(
            read_request(
                &mut request("", r#"{"method":"health"}"#).as_bytes(),
                "secret"
            )
            .unwrap(),
            Command::Health {}
        ));
        assert!(read_request(
            &mut request("", r#"{"method":"health"}"#).as_bytes(),
            "wrong"
        )
        .is_err());
        assert!(read_request(
            &mut request("", r#"{"method":"eval","code":"bad"}"#).as_bytes(),
            "secret"
        )
        .is_err());
        assert!(read_request(
            &mut request("", r#"{"method":"health","extra":1}"#).as_bytes(),
            "secret"
        )
        .is_err());
    }

    #[test]
    fn rejects_browser_requests_and_ambiguous_framing() {
        for headers in [
            "Origin: http://localhost\r\n",
            "Transfer-Encoding: chunked\r\n",
            "Content-Length: 0\r\n",
        ] {
            assert!(read_request(
                &mut request(headers, r#"{"method":"windows"}"#).as_bytes(),
                "secret"
            )
            .is_err());
        }
        assert!(read_request(
            &mut request("", r#"{"method":"health"}"#).as_bytes().take(30),
            "secret"
        )
        .is_err());
        assert!(read_request(&mut request("", &"x".repeat(8193)).as_bytes(), "secret").is_err());
    }
}
