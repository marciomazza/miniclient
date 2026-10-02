//! `browser::Page`/`browser::Element`'s mutation and navigation surface (goto/load/click/fill/
//! trigger/requestSubmit), driven directly by `tokio::test` -- same `MockFetchBackend` the
//! runtime-level navigation suites (`tests/navigation_fetch.rs`, `tests/navigation_trigger.rs`)
//! use, no pyo3/Python involved.

mod common;

use std::sync::Arc;

use _miniclient::browser::Page;
use _miniclient::runtime::Wait;
use common::fetch_mock::MockFetchBackend;

/// Reads back a value through a promise-resolving eval -- `common::EvalExt` can't be used
/// here, its `blocking_recv` panics inside an async `tokio::test`.
async fn eval_str(runtime: &_miniclient::runtime::Runtime, src: &str) -> String {
    let json = runtime
        .send_eval(format!("{{ {src} }}"), Wait::Promise)
        .await
        .expect("channel must not drop")
        .expect("script must not error")
        .expect("script must not return undefined/null");
    serde_json::from_str(&json).expect("result decodes as a string")
}

#[tokio::test]
async fn goto_navigates_and_load_rewrites_the_document_in_place() {
    let page = Page::new("http://localhost/", "{}");
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/next", "<p id=\"a\">hello</p>");
    page.install_fetch_backend(Arc::new(mock)).await;

    page.goto("http://localhost/next").await.unwrap();
    assert_eq!(
        eval_str(page.runtime(), "location.href").await,
        "http://localhost/next"
    );
    let a = page.find("#a", None).await.unwrap().expect("must find #a");
    assert_eq!(a.text().await.unwrap(), "hello");

    // load() rewrites the document in place -- same URL, no navigation.
    page.load("<p id=\"b\">loaded</p>").await.unwrap();
    assert_eq!(
        eval_str(page.runtime(), "location.href").await,
        "http://localhost/next"
    );
    assert!(page.find("#a", None).await.unwrap().is_none());
    let b = page.find("#b", None).await.unwrap().expect("must find #b");
    assert_eq!(b.text().await.unwrap(), "loaded");
}

#[tokio::test]
async fn click_dispatches_an_event_and_fill_sets_the_value() {
    let page = Page::new("http://localhost/", "{}");
    page.install_fetch_backend(Arc::new(MockFetchBackend::new()))
        .await;
    page.load(
        r#"<button id="btn" onclick="this.dataset.clicked = 'yes'">Go</button>
           <input id="inp">"#,
    )
    .await
    .unwrap();

    let btn = page.find("#btn", None).await.unwrap().unwrap();
    btn.click().await.unwrap();
    assert_eq!(btn.attr("data-clicked").await.unwrap(), Some("yes".into()));

    let input = page.find("#inp", None).await.unwrap().unwrap();
    input.fill("hi there").await.unwrap();
    assert_eq!(
        eval_str(page.runtime(), "document.querySelector('#inp').value").await,
        "hi there"
    );
}

#[tokio::test]
async fn trigger_dispatches_an_arbitrary_event_with_the_given_init() {
    let page = Page::new("http://localhost/", "{}");
    page.install_fetch_backend(Arc::new(MockFetchBackend::new()))
        .await;
    page.load(
        r#"<div id="outer"><div id="inner"></div></div>
           <script>
             document.getElementById('outer').addEventListener('bubbling-event', () => {
               document.getElementById('outer').dataset.seen = 'yes';
             });
           </script>"#,
    )
    .await
    .unwrap();
    let outer = page.find("#outer", None).await.unwrap().unwrap();
    let inner = page.find("#inner", None).await.unwrap().unwrap();

    // Explicit `bubbles: false` must override `trigger`'s own default init -- the listener on
    // `outer` never sees it.
    inner
        .trigger(
            "bubbling-event",
            Some(&serde_json::json!({"bubbles": false})),
        )
        .await
        .unwrap();
    assert_eq!(outer.attr("data-seen").await.unwrap(), None);

    // No init given -- the default (`bubbles: true`) lets it reach `outer`.
    inner.trigger("bubbling-event", None).await.unwrap();
    assert_eq!(outer.attr("data-seen").await.unwrap(), Some("yes".into()));
}

#[tokio::test]
async fn request_submit_performs_the_forms_native_navigation() {
    let page = Page::new("http://localhost/", "{}");
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/submitted?x=1", "<p id=\"done\">ok</p>");
    page.install_fetch_backend(Arc::new(mock)).await;
    page.load(
        r#"<form id="f" action="http://localhost/submitted" method="get">
             <input name="x" value="1">
           </form>"#,
    )
    .await
    .unwrap();

    let form = page.find("#f", None).await.unwrap().unwrap();
    form.request_submit().await.unwrap();

    assert_eq!(
        eval_str(page.runtime(), "location.href").await,
        "http://localhost/submitted?x=1"
    );
    assert!(page.find("#done", None).await.unwrap().is_some());
}

/// A hash change queues a timer-driven `hashchange` dispatch; `goto()` right after must not
/// leave the native settle-wait stuck waiting on it.
#[tokio::test]
async fn goto_after_hash_change_does_not_hang() {
    let page = Page::new("http://localhost/a", "{}");
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/a", "<p>/a</p>");
    mock.text("http://localhost/b", "<p>/b</p>");
    page.install_fetch_backend(Arc::new(mock)).await;

    page.goto("http://localhost/a").await.unwrap();
    page.runtime()
        .send_eval("location.hash = '#x'".into(), Wait::Promise)
        .await
        .expect("channel must not drop")
        .expect("script must not error");

    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        page.goto("http://localhost/b"),
    )
    .await;
    outcome
        .expect("goto() hung instead of settling")
        .expect("goto() must not error");

    assert_eq!(
        eval_str(page.runtime(), "location.href").await,
        "http://localhost/b"
    );
}
