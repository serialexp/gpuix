use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures::{AsyncRead, StreamExt};
use gpui::http_client::{http, AsyncBody, HttpClient, Inner, Request, Response, Url};
use web_time::Instant;

const MAX_REQUESTS: usize = 256;
const MAX_PREVIEW: usize = 16 * 1024;
const MAX_HEADERS: usize = 16 * 1024;
const MAX_URL: usize = 2048;

#[derive(Clone, Debug)]
pub(crate) struct Record {
    pub id: u64,
    pub url: String,
    pub method: String,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    pub status: Option<u16>,
    pub started_ms: f64,
    pub headers_ms: Option<f64>,
    pub ended_ms: Option<f64>,
    pub outcome: Option<&'static str>,
    pub request_bytes: usize,
    pub response_bytes: usize,
    request_preview: Vec<u8>,
    response_preview: Vec<u8>,
}

impl Record {
    pub fn kind(&self) -> &'static str {
        let content_type = self
            .response_headers
            .iter()
            .find(|(name, _)| name == "content-type")
            .map(|(_, value)| value.as_str())
            .unwrap_or("");
        if content_type.starts_with("image/") {
            "Img"
        } else if content_type.contains("javascript") {
            "JS"
        } else if content_type.contains("text/css") {
            "CSS"
        } else {
            "Fetch"
        }
    }

    pub fn preview(&self, response: bool) -> String {
        let (bytes, total, headers) = if response {
            (
                &self.response_preview,
                self.response_bytes,
                &self.response_headers,
            )
        } else {
            (
                &self.request_preview,
                self.request_bytes,
                &self.request_headers,
            )
        };
        if total == 0 {
            return "No body captured".into();
        }
        let content_type = headers
            .iter()
            .find(|(name, _)| name == "content-type")
            .map(|(_, value)| value.as_str())
            .unwrap_or("");
        if content_type.contains("json") && total == bytes.len() {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) {
                return serde_json::to_string_pretty(&value).unwrap_or_default();
            }
        }
        match std::str::from_utf8(bytes) {
            Ok(text) if total > bytes.len() => format!("{text}\n… preview truncated at 16 KiB"),
            Ok(text) => text.to_string(),
            Err(_) => format!("Binary body · {total} bytes"),
        }
    }

    pub fn duration(&self, now: f64) -> f64 {
        self.ended_ms.unwrap_or(now) - self.started_ms
    }
}

fn headers(headers: &http::HeaderMap) -> Vec<(String, String)> {
    let mut remaining = MAX_HEADERS;
    headers
        .iter()
        .take_while(|(name, value)| {
            let size = name.as_str().len() + value.as_bytes().len();
            let keep = size <= remaining;
            remaining = remaining.saturating_sub(size);
            keep
        })
        .map(|(name, value)| {
            let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
            (name.to_string(), value)
        })
        .collect()
}

fn recorded_url(uri: &http::Uri) -> String {
    uri.to_string().chars().take(MAX_URL).collect()
}

pub(crate) struct Recorder {
    pub recording: bool,
    pub records: VecDeque<Record>,
    origin: Instant,
    next_id: u64,
    wake: Option<futures::channel::mpsc::Sender<()>>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            recording: true,
            records: VecDeque::new(),
            origin: Instant::now(),
            next_id: 0,
            wake: None,
        }
    }
}

impl Recorder {
    pub fn toggle(&mut self) {
        self.recording = !self.recording;
        self.notify();
    }

    pub fn clear(&mut self) {
        self.records.clear();
        self.notify();
    }

    pub fn now(&self) -> f64 {
        self.origin.elapsed().as_secs_f64() * 1000.0
    }

    fn notify(&mut self) {
        if let Some(wake) = &mut self.wake {
            let _ = wake.try_send(());
        }
    }

    fn start(&mut self, request: &Request<AsyncBody>) -> Option<u64> {
        if !self.recording {
            return None;
        }
        self.next_id += 1;
        let record = Record {
            id: self.next_id,
            url: recorded_url(request.uri()),
            method: request.method().as_str().chars().take(32).collect(),
            request_headers: headers(request.headers()),
            response_headers: Vec::new(),
            status: None,
            started_ms: self.now(),
            headers_ms: None,
            ended_ms: None,
            outcome: None,
            request_bytes: 0,
            response_bytes: 0,
            request_preview: Vec::new(),
            response_preview: Vec::new(),
        };
        if self.records.len() == MAX_REQUESTS {
            self.records.pop_front();
        }
        self.records.push_back(record);
        self.notify();
        Some(self.next_id)
    }

    fn update(&mut self, id: u64, update: impl FnOnce(&mut Record, f64)) {
        let now = self.now();
        if let Some(record) = self.records.iter_mut().find(|record| record.id == id) {
            update(record, now);
            self.notify();
        }
    }

    fn body(&mut self, id: u64, response: bool, bytes: &[u8]) {
        self.update(id, |record, _| {
            let (preview, count) = if response {
                (&mut record.response_preview, &mut record.response_bytes)
            } else {
                (&mut record.request_preview, &mut record.request_bytes)
            };
            *count = count.saturating_add(bytes.len());
            let length = bytes.len().min(MAX_PREVIEW.saturating_sub(preview.len()));
            preview.extend_from_slice(&bytes[..length]);
        });
    }
}

#[derive(Clone)]
pub(crate) struct NetworkLog(pub Arc<Mutex<Recorder>>);
impl gpui::Global for NetworkLog {}

struct RecordingClient {
    client: Arc<dyn HttpClient>,
    log: NetworkLog,
}

struct CompletionGuard {
    log: NetworkLog,
    id: u64,
    active: bool,
}

impl CompletionGuard {
    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for CompletionGuard {
    fn drop(&mut self) {
        if self.active {
            self.log.0.lock().unwrap().update(self.id, |record, now| {
                if record.ended_ms.is_none() {
                    record.ended_ms = Some(now);
                    record.outcome = Some("Cancelled or body not fully consumed");
                }
            });
        }
    }
}

struct BodyTap {
    body: AsyncBody,
    guard: CompletionGuard,
    response: bool,
}

impl AsyncRead for BodyTap {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        if buffer.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let result = Pin::new(&mut self.body).poll_read(cx, buffer);
        match &result {
            Poll::Ready(Ok(length)) => {
                let mut log = self.guard.log.0.lock().unwrap();
                log.body(self.guard.id, self.response, &buffer[..*length]);
                if *length == 0 && self.response && self.guard.active {
                    log.update(self.guard.id, |record, now| {
                        record.ended_ms = Some(now);
                        record.outcome = Some("Complete");
                    });
                }
            }
            Poll::Ready(Err(_)) if self.response => {
                self.guard
                    .log
                    .0
                    .lock()
                    .unwrap()
                    .update(self.guard.id, |record, now| {
                        record.ended_ms = Some(now);
                        record.outcome = Some("Response body failed");
                    });
            }
            _ => (),
        }
        if self.response && matches!(&result, Poll::Ready(Ok(0) | Err(_))) {
            self.guard.disarm();
        }
        result
    }
}

fn tap(body: AsyncBody, log: &NetworkLog, id: u64, response: bool) -> AsyncBody {
    AsyncBody::from_reader(BodyTap {
        body,
        guard: CompletionGuard {
            log: log.clone(),
            id,
            active: response,
        },
        response,
    })
}

impl HttpClient for RecordingClient {
    fn user_agent(&self) -> Option<&http::HeaderValue> {
        self.client.user_agent()
    }
    fn proxy(&self) -> Option<&Url> {
        self.client.proxy()
    }

    fn send(
        &self,
        mut request: Request<AsyncBody>,
    ) -> futures::future::BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let Some(id) = self.log.0.lock().unwrap().start(&request) else {
            return self.client.send(request);
        };
        match &request.body().0 {
            Inner::Bytes(bytes) => self.log.0.lock().unwrap().body(
                id,
                false,
                &bytes.get_ref()[bytes.position() as usize..],
            ),
            Inner::AsyncReader(_) => {
                let body = std::mem::take(request.body_mut());
                *request.body_mut() = tap(body, &self.log, id, false);
            }
            Inner::Empty => (),
        }
        let log = self.log.clone();
        let pending = self.client.send(request);
        let mut guard = CompletionGuard {
            log: log.clone(),
            id,
            active: true,
        };
        Box::pin(async move {
            let result = pending.await;
            match result {
                Ok(mut response) => {
                    log.0.lock().unwrap().update(id, |record, now| {
                        record.status = Some(response.status().as_u16());
                        record.response_headers = headers(response.headers());
                        record.headers_ms = Some(now);
                    });
                    let body = std::mem::take(response.body_mut());
                    let empty = match &body.0 {
                        Inner::Empty => true,
                        Inner::Bytes(bytes) => bytes.position() as usize == bytes.get_ref().len(),
                        Inner::AsyncReader(_) => false,
                    };
                    if empty {
                        log.0.lock().unwrap().update(id, |record, now| {
                            record.ended_ms = Some(now);
                            record.outcome = Some("Complete");
                        });
                        *response.body_mut() = body;
                    } else {
                        *response.body_mut() = tap(body, &log, id, true);
                    }
                    guard.disarm();
                    Ok(response)
                }
                Err(error) => {
                    log.0.lock().unwrap().update(id, |record, now| {
                        record.ended_ms = Some(now);
                        record.outcome = Some("Transport failed");
                    });
                    guard.disarm();
                    Err(error)
                }
            }
        })
    }
}

pub(crate) fn install(cx: &mut gpui::App) {
    if cx.has_global::<NetworkLog>() {
        return;
    }
    let (wake, mut events) = futures::channel::mpsc::channel(1);
    let recorder = Recorder {
        wake: Some(wake),
        ..Recorder::default()
    };
    let log = NetworkLog(Arc::new(Mutex::new(recorder)));
    cx.set_http_client(Arc::new(RecordingClient {
        client: cx.http_client(),
        log: log.clone(),
    }));
    cx.set_global(log);
    cx.spawn(async move |cx| {
        while events.next().await.is_some() {
            cx.update(|cx| cx.refresh_windows());
        }
    })
    .detach();
}

pub(crate) fn log(cx: &gpui::App) -> Option<NetworkLog> {
    cx.try_global::<NetworkLog>().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::AsyncReadExt;

    fn client(inner: Arc<dyn HttpClient>) -> (RecordingClient, NetworkLog) {
        let log = NetworkLog(Arc::new(Mutex::new(Recorder::default())));
        (
            RecordingClient {
                client: inner,
                log: log.clone(),
            },
            log,
        )
    }

    #[test]
    fn recording_preserves_transport_headers_query_and_json() {
        let inner = gpui::http_client::FakeHttpClient::create(|mut request| async move {
            assert_eq!(request.uri(), "https://example.test/api?token=private");
            assert_eq!(request.headers()["authorization"], "Bearer secret");
            let mut body = String::new();
            request.body_mut().read_to_string(&mut body).await.unwrap();
            assert!(body.contains("payload-secret"));
            Ok(Response::builder()
                .status(422)
                .header("content-type", "application/json")
                .header("set-cookie", "session=private")
                .header("location", "https://example.test/next?token=private")
                .header("referer", "https://example.test/start?token=private")
                .body(AsyncBody::from_reader(futures::io::Cursor::new(
                    br#"{"message":"invalid","access_token":"response-secret"}"#.to_vec(),
                )))
                .unwrap())
        });
        let (client, log) = client(inner);
        let request = Request::builder()
            .uri("https://example.test/api?token=private")
            .method("POST")
            .header("authorization", "Bearer secret")
            .header("content-type", "application/json")
            .body(AsyncBody::from(
                r#"{"password":"payload-secret","count":1}"#,
            ))
            .unwrap();
        let mut response = futures::executor::block_on(client.send(request)).unwrap();
        assert_eq!(response.status(), 422);
        let mut body = String::new();
        futures::executor::block_on(response.body_mut().read_to_string(&mut body)).unwrap();
        assert!(body.contains("response-secret"));
        let records = &log.0.lock().unwrap().records;
        let record = &records[0];
        assert_eq!(record.url, "https://example.test/api?token=private");
        assert_eq!(
            record
                .request_headers
                .iter()
                .find(|(name, _)| name == "authorization")
                .unwrap()
                .1,
            "Bearer secret"
        );
        assert_eq!(
            record
                .response_headers
                .iter()
                .find(|(name, _)| name == "set-cookie")
                .unwrap()
                .1,
            "session=private"
        );
        assert!(record.preview(false).contains("payload-secret"));
        assert!(record.preview(true).contains("response-secret"));
        for name in ["location", "referer"] {
            assert!(record
                .response_headers
                .iter()
                .any(|(header, value)| header == name && value.contains("token=private")));
        }
        let mut preview = record.clone();
        for (content_type, body) in [
            (
                "application/x-www-form-urlencoded",
                "password=visible&token=visible",
            ),
            ("application/json", "{\"password\":\"visible\""),
            ("", "token=visible"),
        ] {
            preview.request_headers = vec![("content-type".into(), content_type.into())];
            preview.request_preview = body.as_bytes().to_vec();
            preview.request_bytes = body.len();
            assert_eq!(preview.preview(false), body);
        }
        preview.request_headers = vec![("content-type".into(), "application/json".into())];
        preview.request_preview = b"{\"token\":\"visible".to_vec();
        preview.request_bytes = MAX_PREVIEW + 1;
        assert!(preview.preview(false).starts_with("{\"token\":\"visible"));
        assert!(preview.preview(false).contains("truncated"));
        assert_eq!(record.status, Some(422));
        assert_eq!(record.outcome, Some("Complete"));
        assert_eq!(record.response_bytes, body.len());
        assert!(record.headers_ms.unwrap() <= record.ended_ms.unwrap());
    }

    #[test]
    fn recording_is_bounded_pause_skips_new_requests_and_clear_does_not_resurrect() {
        let mut recorder = Recorder::default();
        for _ in 0..MAX_REQUESTS + 5 {
            recorder.start(
                &Request::builder()
                    .uri("http://example.test/")
                    .body(AsyncBody::empty())
                    .unwrap(),
            );
        }
        assert_eq!(recorder.records.len(), MAX_REQUESTS);
        assert_eq!(recorder.records[0].id, 6);
        let id = recorder.records.back().unwrap().id;
        recorder.body(id, true, &vec![b'x'; MAX_PREVIEW * 3]);
        assert_eq!(
            recorder.records.back().unwrap().response_preview.len(),
            MAX_PREVIEW
        );
        assert_eq!(
            recorder.records.back().unwrap().response_bytes,
            MAX_PREVIEW * 3
        );
        recorder.toggle();
        assert!(!recorder.recording);
        assert!(recorder.start(&Request::new(AsyncBody::empty())).is_none());
        recorder.clear();
        recorder.body(id, true, b"late body");
        assert!(recorder.records.is_empty());
    }

    #[test]
    fn dropped_future_and_unconsumed_body_finish_and_transport_errors_are_preserved() {
        let (client, log) = client(gpui::http_client::FakeHttpClient::create(|_| async {
            Ok(Response::new(AsyncBody::from_reader(
                futures::io::Cursor::new(b"hello".to_vec()),
            )))
        }));
        let pending = client.send(
            Request::builder()
                .uri("http://example.test/")
                .body(AsyncBody::empty())
                .unwrap(),
        );
        assert!(log.0.lock().unwrap().records[0].ended_ms.is_none());
        drop(pending);
        assert!(log.0.lock().unwrap().records[0].ended_ms.is_some());
        let response = futures::executor::block_on(
            client.send(
                Request::builder()
                    .uri("http://example.test/")
                    .body(AsyncBody::empty())
                    .unwrap(),
            ),
        )
        .unwrap();
        assert!(log.0.lock().unwrap().records[1].ended_ms.is_none());
        drop(response);
        assert!(log.0.lock().unwrap().records[1].ended_ms.is_some());
        let (client, log) = self::client(gpui::http_client::FakeHttpClient::create(|_| async {
            anyhow::bail!("original error")
        }));
        let result = futures::executor::block_on(
            client.send(
                Request::builder()
                    .uri("http://example.test/")
                    .body(AsyncBody::empty())
                    .unwrap(),
            ),
        );
        assert_eq!(result.err().unwrap().to_string(), "original error");
        assert_eq!(
            log.0.lock().unwrap().records[0].outcome,
            Some("Transport failed")
        );
    }

    #[test]
    fn streaming_request_body_is_not_eagerly_buffered_or_modified() {
        let (client, log) = client(gpui::http_client::FakeHttpClient::create(
            |mut request| async move {
                let mut bytes = Vec::new();
                request.body_mut().read_to_end(&mut bytes).await.unwrap();
                assert_eq!(bytes, b"a\0b");
                Ok(Response::new(AsyncBody::empty()))
            },
        ));
        let request = Request::builder()
            .uri("http://example.test/")
            .body(AsyncBody::from_reader(futures::io::Cursor::new(
                b"a\0b".to_vec(),
            )))
            .unwrap();
        let pending = client.send(request);
        assert_eq!(log.0.lock().unwrap().records[0].request_bytes, 0);
        let mut response = futures::executor::block_on(pending).unwrap();
        futures::executor::block_on(response.body_mut().read_to_end(&mut Vec::new())).unwrap();
        let log = log.0.lock().unwrap();
        assert_eq!(log.records[0].request_bytes, 3);
        assert_eq!(log.records[0].outcome, Some("Complete"));
    }
}
