//! A `FetchBackend` that matches by method + regex (like `httpx_mock`), so the vendored htmx
//! JS test suites can drive real `fetch()` calls entirely inside `cargo test`.
//! `tests/htmx_fetch_mock_bridge.js` only expects four `__mini_fm_*` globals to exist, however
//! they got bound.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use _miniclient::ops::{FetchBackend, FetchRequest, FetchResponse};
use _miniclient::runtime::Runtime;
use async_trait::async_trait;
use deno_error::JsErrorBox;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

#[derive(Deserialize)]
struct RegisterReq {
    method: String,
    #[serde(rename = "urlPattern")]
    url_pattern: String,
    #[serde(default)]
    body: String,
    #[serde(default = "default_status")]
    status: u16,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    once: bool,
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    error_msg: String,
}

fn default_status() -> u16 {
    200
}

#[derive(Deserialize)]
struct NextReq {
    seq_id: u32,
}

struct MockEntry {
    method: String,
    pattern: Regex,
    body: String,
    status: u16,
    headers: HashMap<String, String>,
    once: bool,
    used: bool,
    is_error: bool,
    error_msg: String,
}

/// A sequential-response entry always answers with the same configured body -- `next()` only
/// releases the gate a pending `fetch()` is waiting on before it dispatches.
struct SeqEntry {
    method: String,
    pattern: Regex,
    body: String,
    status: u16,
    headers: HashMap<String, String>,
    tx: mpsc::UnboundedSender<()>,
    rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<()>>,
}

#[derive(Default)]
struct Inner {
    entries: Mutex<Vec<MockEntry>>,
    seq_entries: Mutex<HashMap<u32, Arc<SeqEntry>>>,
    next_seq_id: Mutex<u32>,
    /// Mirrors `MockFetchBackend`'s `inflight` map: `op_fetch_abort` drops the sender here to
    /// wake a `fetch()` stuck waiting on a sequential entry's release gate.
    inflight: Mutex<HashMap<String, oneshot::Sender<()>>>,
}

#[derive(Clone, Default)]
pub struct HtmxFetchMock(Arc<Inner>);

impl HtmxFetchMock {
    pub fn new() -> Self {
        Self::default()
    }

    fn find_seq_match(&self, method: &str, url: &str) -> Option<Arc<SeqEntry>> {
        self.0
            .seq_entries
            .lock()
            .expect("lock poisoned")
            .values()
            .find(|e| e.method == method && e.pattern.is_match(url))
            .cloned()
    }

    fn dispatch(&self, method: &str, url: &str) -> Result<FetchResponse, JsErrorBox> {
        if let Some(entry) = self.find_seq_match(method, url) {
            return Ok(FetchResponse {
                status: entry.status,
                status_text: String::new(),
                headers: entry
                    .headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                body: Some(entry.body.clone().into_bytes().into()),
                url: url.to_string(),
            });
        }
        let mut entries = self.0.entries.lock().expect("lock poisoned");
        for entry in entries.iter_mut().rev() {
            if entry.method != method || !entry.pattern.is_match(url) {
                continue;
            }
            if entry.once {
                if entry.used {
                    continue;
                }
                entry.used = true;
            }
            if entry.is_error {
                return Err(JsErrorBox::generic(entry.error_msg.clone()));
            }
            return Ok(FetchResponse {
                status: entry.status,
                status_text: String::new(),
                headers: entry
                    .headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                body: Some(entry.body.clone().into_bytes().into()),
                url: url.to_string(),
            });
        }
        Err(JsErrorBox::generic(format!(
            "mock: no route for {method} {url}"
        )))
    }

    fn op_register(&self, args: &[Value]) -> Value {
        let req: RegisterReq =
            serde_json::from_value(args[0].clone()).expect("valid __mini_fm_register payload");
        let pattern = Regex::new(&req.url_pattern).expect("valid regex from the JS bridge");
        self.0
            .entries
            .lock()
            .expect("lock poisoned")
            .push(MockEntry {
                method: req.method.to_uppercase(),
                pattern,
                body: req.body,
                status: req.status,
                headers: req.headers,
                once: req.once,
                used: false,
                is_error: req.is_error,
                error_msg: req.error_msg,
            });
        Value::Object(Default::default())
    }

    fn op_reset(&self, _args: &[Value]) -> Value {
        self.0.entries.lock().expect("lock poisoned").clear();
        self.0.seq_entries.lock().expect("lock poisoned").clear();
        Value::Object(Default::default())
    }

    fn op_register_seq(&self, args: &[Value]) -> Value {
        let req: RegisterReq =
            serde_json::from_value(args[0].clone()).expect("valid __mini_fm_register_seq payload");
        let pattern = Regex::new(&req.url_pattern).expect("valid regex from the JS bridge");
        let (tx, rx) = mpsc::unbounded_channel();
        let mut next_id = self.0.next_seq_id.lock().expect("lock poisoned");
        let id = *next_id;
        *next_id += 1;
        drop(next_id);
        self.0.seq_entries.lock().expect("lock poisoned").insert(
            id,
            Arc::new(SeqEntry {
                method: req.method.to_uppercase(),
                pattern,
                body: req.body,
                status: req.status,
                headers: req.headers,
                tx,
                rx: tokio::sync::Mutex::new(rx),
            }),
        );
        Value::from(id)
    }

    fn op_next(&self, args: &[Value]) -> Value {
        let req: NextReq =
            serde_json::from_value(args[0].clone()).expect("valid __mini_fm_next payload");
        if let Some(entry) = self
            .0
            .seq_entries
            .lock()
            .expect("lock poisoned")
            .get(&req.seq_id)
        {
            entry.tx.send(()).ok();
        }
        Value::Object(Default::default())
    }

    /// Binds the four `__mini_fm_*` globals `htmx_fetch_mock_bridge.js` expects.
    pub fn install(&self, rt: &Runtime) {
        for (name, f) in [
            (
                "__mini_fm_register",
                Box::new({
                    let this = self.clone();
                    move |args: Vec<Value>| this.op_register(&args)
                }) as Box<dyn Fn(Vec<Value>) -> Value + Send>,
            ),
            (
                "__mini_fm_reset",
                Box::new({
                    let this = self.clone();
                    move |args: Vec<Value>| this.op_reset(&args)
                }),
            ),
            (
                "__mini_fm_register_seq",
                Box::new({
                    let this = self.clone();
                    move |args: Vec<Value>| this.op_register_seq(&args)
                }),
            ),
            (
                "__mini_fm_next",
                Box::new({
                    let this = self.clone();
                    move |args: Vec<Value>| this.op_next(&args)
                }),
            ),
        ] {
            rt.send_register_rust_function(name.to_string(), f)
                .blocking_recv()
                .expect("isolate thread answered")
                .unwrap_or_else(|e| panic!("{name} bound: {e:?}"));
        }
    }
}

#[async_trait(?Send)]
impl FetchBackend for HtmxFetchMock {
    async fn fetch(&self, req: FetchRequest) -> Result<FetchResponse, JsErrorBox> {
        let method = req.method.to_uppercase();
        // Gate on a matching sequential entry's release queue before dispatching, so
        // `fetchMock.mockSequentialResponses(...).next()` controls exactly when a pending
        // request resolves. Raced against `abort()` so `htmx:abort` cancels a still-gated
        // request instead of leaving it to resolve once released.
        if let Some(entry) = self.find_seq_match(&method, &req.url) {
            let (tx, rx) = oneshot::channel();
            self.0
                .inflight
                .lock()
                .expect("lock poisoned")
                .insert(req.id.clone(), tx);
            let mut gate = entry.rx.lock().await;
            // `biased`: abort must win a same-tick race against a release that fires right
            // after it (htmx's own abort test calls `seq.next()` immediately after triggering
            // `htmx:abort`) -- abort has to preempt whatever `fetch()` was waiting on, not
            // race it.
            let outcome = tokio::select! {
                biased;
                _ = rx => Err(JsErrorBox::generic("mock: request aborted")),
                _ = gate.recv() => Ok(()),
            };
            drop(gate);
            self.0
                .inflight
                .lock()
                .expect("lock poisoned")
                .remove(&req.id);
            outcome?;
        }
        self.dispatch(&method, &req.url)
    }

    fn fetch_sync(&self, _req: FetchRequest) -> Result<FetchResponse, JsErrorBox> {
        // htmx 4 only ever calls `fetch()` (see htmx-guidance.md) -- no vendored unit test
        // reaches this path.
        Err(JsErrorBox::generic("HtmxFetchMock: no sync backend"))
    }

    fn abort(&self, request_id: &str) -> Result<(), JsErrorBox> {
        if let Some(tx) = self
            .0
            .inflight
            .lock()
            .expect("lock poisoned")
            .remove(request_id)
        {
            tx.send(()).ok();
        }
        Ok(())
    }
}
