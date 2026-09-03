mod common;

use common::EvalExt;
use common::fetch_mock::{MockFetchBackend, runtime_with_mock};
use serde_json::{Value, json};

#[test]
fn fetch_reads_text() {
    let mock = MockFetchBackend::new();
    mock.text("http://api.example.com/hello", "hello world");
    let rt = runtime_with_mock(&mock);
    assert_eq!(
        rt.eval_async::<String>("fetch('http://api.example.com/hello').then(r => r.text())"),
        "hello world",
    );
}

#[test]
fn fetch_reads_json() {
    let mock = MockFetchBackend::new();
    mock.json(
        "http://api.example.com/data",
        json!({ "name": "Alice", "age": 30 }),
    );
    let rt = runtime_with_mock(&mock);
    assert_eq!(
        rt.eval_async::<Value>("fetch('http://api.example.com/data').then(r => r.json())"),
        json!({"name": "Alice", "age": 30}),
    );
}

#[test]
fn fetch_status_ok() {
    let mock = MockFetchBackend::new();
    mock.status("http://api.example.com/ok", 200);
    let rt = runtime_with_mock(&mock);
    assert_eq!(
        rt.eval_async::<u16>("fetch('http://api.example.com/ok').then(r => r.status)"),
        200,
    );
}

#[test]
fn fetch_does_not_throw_on_404() {
    let mock = MockFetchBackend::new();
    mock.status("http://api.example.com/missing", 404);
    let rt = runtime_with_mock(&mock);
    assert_eq!(
        rt.eval_async::<Value>(
            "fetch('http://api.example.com/missing').then(r => ({ok: r.ok, status: r.status}))",
        ),
        json!({"ok": false, "status": 404}),
    );
}

#[test]
fn fetch_follows_redirect() {
    let mock = MockFetchBackend::new();
    mock.redirect("http://api.example.com/old", "http://api.example.com/new");
    mock.text("http://api.example.com/new", "moved");
    let rt = runtime_with_mock(&mock);
    assert_eq!(
        rt.eval_async::<Value>(
            "fetch('http://api.example.com/old').then(async r => ({
               url: r.url, status: r.status, body: await r.text(),
             }))",
        ),
        json!({"url": "http://api.example.com/new", "status": 200, "body": "moved"}),
    );
}

#[test]
fn fetch_uses_the_requested_method() {
    for method in ["GET", "POST", "PUT", "DELETE", "PATCH"] {
        let mock = MockFetchBackend::new();
        mock.text("http://api.example.com/resource", method);
        let rt = runtime_with_mock(&mock);
        let body = rt.eval_async::<String>(&format!(
            "fetch('http://api.example.com/resource', {{method: '{method}'}}).then(r =>
  r.text(),
)",
        ));
        assert_eq!(body, method);
        assert_eq!(mock.last_request().method, method);
    }
}

#[test]
fn fetch_forwards_custom_request_headers() {
    let mock = MockFetchBackend::new();
    mock.text("http://api.example.com/auth", "ok");
    let rt = runtime_with_mock(&mock);
    rt.eval_async::<String>(
        "fetch('http://api.example.com/auth', {
  headers: {Authorization: 'Bearer token123'},
}).then(r => r.text())",
    );
    assert_eq!(
        mock.last_request().header("authorization"),
        Some("Bearer token123"),
    );
}

#[test]
fn fetch_exposes_response_headers_to_js() {
    let mock = MockFetchBackend::new();
    mock.reply(
        "http://api.example.com/typed",
        200,
        &[("content-type", "application/json; charset=utf-8")],
        b"{}",
    );
    let rt = runtime_with_mock(&mock);
    let ct = rt.eval_async::<String>(
        "fetch('http://api.example.com/typed').then(r => r.headers.get('content-type'))",
    );
    assert!(ct.contains("application/json"), "content-type was {ct:?}");
}

#[test]
fn fetch_sends_a_post_json_body() {
    let mock = MockFetchBackend::new();
    mock.text("http://api.example.com/echo", "saved");
    let rt = runtime_with_mock(&mock);
    let body = rt.eval_async::<String>(
        "fetch('http://api.example.com/echo', {
  method: 'POST',
  headers: {'content-type': 'application/json'},
  body: JSON.stringify({key: 'value'}),
}).then(r => r.text())",
    );
    assert_eq!(body, "saved");
    let request = mock.last_request();
    assert_eq!(request.method, "POST");
    assert_eq!(request.json_body(), json!({"key": "value"}));
}

#[test]
fn fetch_handles_an_empty_body() {
    let mock = MockFetchBackend::new();
    mock.reply("http://api.example.com/empty", 204, &[], b"");
    let rt = runtime_with_mock(&mock);
    assert_eq!(
        rt.eval_async::<u16>("fetch('http://api.example.com/empty').then(r => r.status)"),
        204,
    );
}

#[test]
fn fetch_abort_cancels_an_in_flight_request() {
    let mock = MockFetchBackend::new();
    mock.hang("http://api.example.com/slow");
    let rt = runtime_with_mock(&mock);
    let outcome = rt.eval_async::<String>(
        r#"
        (async () => {
          const controller = new AbortController();
          const p = fetch('http://api.example.com/slow', {signal: controller.signal});
          // Yield so op_fetch is dispatched and the mock registers it in-flight before abort.
          await new Promise(r => setTimeout(r, 5));
          controller.abort();
          return await Promise.race([
            p.then(
              () => 'resolved',
              e => e.name,
            ),
            new Promise(r => setTimeout(() => r('hung'), 2000)),
          ]);
        })();"#,
    );
    assert_eq!(outcome, "AbortError");
    // The backend request was actually ended by the abort, not left running.
    assert_eq!(mock.aborted().len(), 1);
}
