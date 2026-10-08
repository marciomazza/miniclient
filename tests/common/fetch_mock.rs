//! In-memory `FetchBackend` for the ported fetch suites -- no sockets, no HTTP crate, no
//! Python. Routes are keyed by exact URL (like `httpx_mock.add_response(url=...)`); every
//! request is recorded for assertions; a `hang` route stays pending until `abort()` ends it,
//! mirroring `fetch.py`'s in-flight `pending` map.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use _miniclient::ops::{FetchBackend, FetchRequest, FetchResponse};
use _miniclient::runtime::Runtime;
use async_trait::async_trait;
use deno_error::JsErrorBox;
use serde_json::Value;
use tokio::sync::oneshot;

#[derive(Clone)]
enum Route {
    Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    Hang,
}

#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub url: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl RecordedRequest {
    /// Case-insensitive header lookup -- `Headers` lowercases names per spec, but assert on
    /// intent, not that quirk.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json_body(&self) -> Value {
        serde_json::from_slice(&self.body).expect("request body is JSON")
    }
}

#[derive(Default)]
struct Inner {
    routes: Mutex<HashMap<String, Route>>,
    requests: Mutex<Vec<RecordedRequest>>,
    inflight: Mutex<HashMap<String, oneshot::Sender<()>>>,
    aborted: Mutex<Vec<String>>,
}

#[derive(Clone, Default)]
pub struct MockFetchBackend(Arc<Inner>);

impl MockFetchBackend {
    pub fn new() -> Self {
        Self::default()
    }

    fn route(&self, url: &str, route: Route) -> &Self {
        self.0
            .routes
            .lock()
            .expect("mock lock poisoned")
            .insert(url.to_string(), route);
        self
    }

    pub fn reply(&self, url: &str, status: u16, headers: &[(&str, &str)], body: &[u8]) -> &Self {
        self.route(
            url,
            Route::Reply {
                status,
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                body: body.to_vec(),
            },
        )
    }

    pub fn text(&self, url: &str, body: &str) -> &Self {
        self.reply(url, 200, &[("content-type", "text/plain")], body.as_bytes())
    }

    pub fn json(&self, url: &str, body: Value) -> &Self {
        self.reply(
            url,
            200,
            &[("content-type", "application/json")],
            body.to_string().as_bytes(),
        )
    }

    pub fn status(&self, url: &str, status: u16) -> &Self {
        self.reply(url, status, &[], b"")
    }

    pub fn redirect(&self, url: &str, location: &str) -> &Self {
        self.reply(url, 302, &[("location", location)], b"")
    }

    /// The answer to the CORS preflight (`OPTIONS`) of `url`, apart from its actual response.
    pub fn preflight(&self, url: &str, headers: &[(&str, &str)]) -> &Self {
        self.reply(&format!("OPTIONS {url}"), 204, headers, b"")
    }

    pub fn hang(&self, url: &str) -> &Self {
        self.route(url, Route::Hang)
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.0.requests.lock().expect("mock lock poisoned").clone()
    }

    pub fn last_request(&self) -> RecordedRequest {
        self.0
            .requests
            .lock()
            .expect("mock lock poisoned")
            .last()
            .cloned()
            .expect("a request was made")
    }

    pub fn aborted(&self) -> Vec<String> {
        self.0.aborted.lock().expect("mock lock poisoned").clone()
    }
}

#[async_trait(?Send)]
impl FetchBackend for MockFetchBackend {
    async fn fetch(&self, req: FetchRequest) -> Result<FetchResponse, JsErrorBox> {
        self.0
            .requests
            .lock()
            .expect("mock lock poisoned")
            .push(RecordedRequest {
                url: req.url.clone(),
                method: req.method.clone(),
                headers: req.headers.clone(),
                body: req.body.as_deref().unwrap_or_default().to_vec(),
            });

        let route = {
            let routes = self.0.routes.lock().expect("mock lock poisoned");
            let preflight = format!("OPTIONS {}", req.url);
            routes
                .get(&preflight)
                .filter(|_| req.method == "OPTIONS")
                .or_else(|| routes.get(&req.url))
                .cloned()
        }
        .ok_or_else(|| JsErrorBox::generic(format!("mock: no route for {}", req.url)))?;
        match route {
            Route::Hang => {
                let (tx, rx) = oneshot::channel();
                self.0
                    .inflight
                    .lock()
                    .expect("mock lock poisoned")
                    .insert(req.id.clone(), tx);
                // Wakes with an error the moment abort() drops the sender.
                let _ = rx.await;
                self.0
                    .aborted
                    .lock()
                    .expect("mock lock poisoned")
                    .push(req.id.clone());
                Err(JsErrorBox::generic("mock: request aborted"))
            }
            Route::Reply {
                status,
                mut headers,
                body,
            } => {
                // A server open to any origin unless a route says otherwise, so tests of fetch
                // mechanics don't depend on the page's own origin.
                let preflight = req.method == "OPTIONS";
                for (name, default) in [
                    ("access-control-allow-origin", "*"),
                    ("access-control-allow-methods", "*"),
                    ("access-control-allow-headers", "*, authorization"),
                ] {
                    // Only the origin applies to every response; the others answer preflights.
                    let applies = preflight || name.ends_with("origin");
                    if applies && !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name)) {
                        headers.push((name.into(), default.into()));
                    }
                }
                Ok(FetchResponse {
                    status,
                    status_text: String::new(),
                    headers,
                    body: Some(body.into()),
                    url: req.url,
                })
            }
        }
    }

    fn abort(&self, request_id: &str) -> Result<(), JsErrorBox> {
        // Dropping the sender wakes the pending `rx.await` in `fetch`.
        self.0
            .inflight
            .lock()
            .expect("mock lock poisoned")
            .remove(request_id);
        Ok(())
    }
}

/// A runtime with the mock installed as its fetch backend.
pub fn runtime_with_mock(mock: &MockFetchBackend) -> Runtime {
    let rt = Runtime::new("{}");
    rt.send_install_fetch_backend(Arc::new(mock.clone()))
        .blocking_recv()
        .expect("fetch backend installed");
    rt
}
