use futures::future::{AbortRegistration, Abortable};
use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, Request};
use std::sync::Arc;

pub(crate) const MAX_BODY: usize = 8 * 1024 * 1024;

pub(crate) struct FetchRequest {
    pub(crate) id: u64,
    pub(crate) request: Request<AsyncBody>,
    pub(crate) registration: AbortRegistration,
}

pub(crate) struct FetchResponse {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, Vec<u8>)>,
    pub(crate) body: Vec<u8>,
}

pub(crate) struct FetchCompletion {
    pub(crate) id: u64,
    pub(crate) result: Result<FetchResponse, String>,
}

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
