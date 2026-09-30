//! the host-state container (`Page`) and its explicit `JsRuntime` navigate primitive.
//! No navigation trigger involved -- these tests drive `Page::navigate` directly and assert every
//! piece of host-owned state (cookies, session history, `FetchBackend`, `RustFunctions`) is
//! present and working in the new realm.

mod common;

use std::sync::{Arc, Mutex};

use _miniclient::runtime::Page;
use common::EvalExt;
use common::fetch_mock::MockFetchBackend;

#[test]
fn navigate_carries_cookies_forward() {
    let mut page = Page::new("http://localhost/", "{}");
    page.runtime().run("document.cookie = 'a=1'; void 0");

    page.navigate("http://localhost/");

    assert_eq!(page.runtime().eval::<String>("document.cookie"), "a=1");
}

#[test]
fn navigate_carries_session_history_forward() {
    let mut page = Page::new("http://localhost/", "{}");
    page.runtime().run("history.pushState({n: 1}, '', '/next')");

    page.navigate("http://localhost/");

    assert_eq!(page.runtime().eval::<i32>("history.length"), 2);
    assert_eq!(page.runtime().eval::<i32>("history.state.n"), 1);
}

#[test]
fn navigate_reinstalls_the_fetch_backend() {
    let mut page = Page::new("http://localhost/", "{}");
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/ok", "hello");
    page.install_fetch_backend(Arc::new(mock));

    page.navigate("http://localhost/");

    let body: String = page
        .runtime()
        .eval_async("fetch('http://localhost/ok').then(r => r.text())");
    assert_eq!(body, "hello");
}

#[test]
fn navigate_replays_rust_function_bindings() {
    let mut page = Page::new("http://localhost/", "{}");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    page.register_rust_function(
        "__testFn".into(),
        Arc::new(move |args: Vec<serde_json::Value>| {
            recorded.lock().unwrap().push(args.clone());
            serde_json::json!("ok")
        }),
    );

    page.navigate("http://localhost/");

    assert_eq!(page.runtime().eval::<String>("__testFn(42)"), "ok");
    assert_eq!(calls.lock().unwrap().len(), 1);
}
