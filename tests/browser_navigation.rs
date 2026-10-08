//! `browser::Page`/`browser::Element`'s mutation and navigation surface (goto/load/click/fill/
//! trigger/requestSubmit), driven directly by `tokio::test` -- same `MockFetchBackend` the
//! runtime-level navigation suites (`tests/navigation_fetch.rs`, `tests/navigation_trigger.rs`)
//! use, no pyo3/Python involved.

mod common;

use std::sync::Arc;

use _miniclient::browser::Browser;
use _miniclient::runtime::Wait;
use common::fetch_mock::MockFetchBackend;
use common::{aeval_async, arun_async};

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
    let page = Browser::new("{}", false).new_page();
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
    let page = Browser::new("{}", false).new_page();
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
    let page = Browser::new("{}", false).new_page();
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
    let page = Browser::new("{}", false).new_page();
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

#[tokio::test]
async fn request_submit_post_sends_the_forms_encoded_body() {
    let page = Browser::new("{}", false).new_page();
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/action", "<body><p>done</p></body>");
    page.install_fetch_backend(Arc::new(mock.clone())).await;
    page.load(
        r#"<form method="post" action="/action"><input name="x" value="42">
           <button type="submit" id="btn">go</button></form>"#,
    )
    .await
    .unwrap();

    let form = page.find("form", None).await.unwrap().unwrap();
    assert_eq!(form.tag_name().await.unwrap(), "FORM");
    form.request_submit().await.unwrap();

    let request = mock.last_request();
    assert_eq!(request.method, "POST");
    assert_eq!(request.body, b"x=42");
    assert_eq!(
        request.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    let el = page.find("p", None).await.unwrap().unwrap();
    assert_eq!(el.text().await.unwrap(), "done");
}

/// `click()`/`trigger()`/`fill()` wait for the page to settle, even for a plain (non-htmx)
/// `fetch()`-driven handler.
#[tokio::test]
async fn click_awaits_a_plain_fetch_settle() {
    let page = Browser::new("{}", false).new_page();
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/data", "done");
    page.install_fetch_backend(Arc::new(mock)).await;
    page.load(
        r#"<button id="btn">go</button>
           <div id="out"></div>
           <script>
               document.getElementById("btn").addEventListener("click", async () => {
                   const res = await fetch("/data");
                   document.getElementById("out").textContent = await res.text();
               });
           </script>"#,
    )
    .await
    .unwrap();

    let btn = page.find("#btn", None).await.unwrap().unwrap();
    btn.click().await.unwrap();
    let out = page.find("#out", None).await.unwrap().unwrap();
    assert_eq!(out.text().await.unwrap(), "done");
}

#[tokio::test]
async fn goto_loads_head_and_body() {
    let page = Browser::new("{}", false).new_page();
    let mock = MockFetchBackend::new();
    mock.text(
        "http://localhost/page",
        "<html><head><title>T</title></head><body><span id='s'>ok</span></body></html>",
    );
    page.install_fetch_backend(Arc::new(mock)).await;

    page.goto("http://localhost/page").await.unwrap();
    assert_eq!(eval_str(page.runtime(), "document.title").await, "T");
    let s = page.find("#s", None).await.unwrap().unwrap();
    assert_eq!(s.inner_html().await.unwrap(), "ok");
}

/// A failed fetch must not error out of `goto()` -- the new realm is already committed and the
/// old document is gone, so `goto()` surfaces the failure as an error-state document, same as a
/// real browser's top-level navigation. The error document's own shape is covered by
/// `tests/navigation_fetch.rs`; this only checks `browser::Page::goto` doesn't propagate an
/// `Err` for it.
#[tokio::test]
async fn goto_commits_an_error_document_on_failed_navigation() {
    let page = Browser::new("{}", false).new_page();
    let mock = MockFetchBackend::new();
    // No route registered for /unreachable -- the mock answers every unregistered URL with an
    // error, same effect as a real connection failure.
    page.install_fetch_backend(Arc::new(mock)).await;

    page.goto("http://localhost/unreachable").await.unwrap();
    assert!(
        page.find("[data-mini-navigation-error]", None)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn goto_navigates_across_domains() {
    let page = Browser::new("{}", false).new_page();
    let mock = MockFetchBackend::new();
    mock.text(
        "http://localhost/start",
        "<html><body><p>start</p></body></html>",
    );
    mock.text(
        "http://example.com/other",
        "<html><body><p>other</p></body></html>",
    );
    page.install_fetch_backend(Arc::new(mock)).await;

    page.goto("http://localhost/start").await.unwrap();
    assert_eq!(
        eval_str(page.runtime(), "location.href").await,
        "http://localhost/start"
    );
    assert_eq!(
        eval_str(page.runtime(), "document.baseURI").await,
        "http://localhost/start"
    );

    page.goto("http://example.com/other").await.unwrap();
    assert_eq!(
        eval_str(page.runtime(), "location.href").await,
        "http://example.com/other"
    );
    assert_eq!(
        eval_str(page.runtime(), "document.baseURI").await,
        "http://example.com/other"
    );
}

async fn assert_goto_resolves(initial_url: Option<&str>, relative_url: &str, expected_url: &str) {
    let page = Browser::new("{}", false).new_page();
    let mock = MockFetchBackend::new();
    if let Some(initial_url) = initial_url {
        mock.text(initial_url, "<html><body><p>start</p></body></html>");
    }
    mock.text(expected_url, "<html><body><p>target</p></body></html>");
    page.install_fetch_backend(Arc::new(mock)).await;

    if let Some(initial_url) = initial_url {
        page.goto(initial_url).await.unwrap();
    }
    page.goto(relative_url).await.unwrap();
    assert_eq!(
        eval_str(page.runtime(), "location.href").await,
        expected_url
    );
}

#[tokio::test]
async fn goto_resolves_relative_to_current_page() {
    assert_goto_resolves(
        Some("http://localhost/start/page"),
        "other",
        "http://localhost/start/other",
    )
    .await;
}

#[tokio::test]
async fn goto_resolves_root_relative_after_a_prior_navigation() {
    assert_goto_resolves(
        Some("http://localhost/start/page"),
        "/",
        "http://localhost/",
    )
    .await;
}

#[tokio::test]
async fn goto_resolves_root_relative_with_no_prior_navigation() {
    assert_goto_resolves(None, "/", "http://localhost/").await;
}

/// A script's global writes and pending timers from one page must not survive navigation to
/// the next: `goto()` boots a brand new isolate, so `globalThis` starts from a clean baseline
/// rather than being reset in place.
#[tokio::test]
async fn goto_isolates_globals_and_timers_across_navigations() {
    let page = Browser::new("{}", false).new_page();
    let mock = MockFetchBackend::new();
    mock.text(
        "http://localhost/a",
        r#"<html><body><script>
            window.__foo = 1;
            setTimeout(() => { window.__leaked = true; }, 20);
        </script></body></html>"#,
    );
    mock.text("http://localhost/b", "<html><body><p>b</p></body></html>");
    page.install_fetch_backend(Arc::new(mock)).await;

    page.goto("http://localhost/a").await.unwrap();
    let foo: i64 = aeval_async(page.runtime(), "window.__foo").await;
    assert_eq!(foo, 1);

    page.goto("http://localhost/b").await.unwrap();
    assert_eq!(
        eval_str(page.runtime(), "typeof window.__foo").await,
        "undefined"
    );

    // Give page A's pending timer a chance to fire; goto() must have cleared it.
    page.runtime()
        .send_eval(
            "new Promise(resolve => setTimeout(resolve, 50))".into(),
            Wait::Promise,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        eval_str(page.runtime(), "typeof window.__leaked").await,
        "undefined"
    );
}

/// Each full navigation replaces the whole jsdom `Window`, and with it
/// `window._sessionHistory` -- `history.back()` must still reach the previous page, same as a
/// real browser.
#[tokio::test]
async fn history_back_and_forward_span_full_navigations() {
    let page = Browser::new("{}", false).new_page();
    let mock = MockFetchBackend::new();
    // history.back()/forward() re-fetch each URL (no document cache, see
    // __mini.navigateHistory), so the same route must answer more than once.
    mock.text("http://localhost/a", "<html><body><p>a</p></body></html>");
    mock.text("http://localhost/b", "<html><body><p>b</p></body></html>");
    page.install_fetch_backend(Arc::new(mock)).await;

    page.goto("http://localhost/a").await.unwrap();
    page.goto("http://localhost/b").await.unwrap();
    assert_eq!(
        eval_str(page.runtime(), "location.href").await,
        "http://localhost/b"
    );
    let length: i64 = aeval_async(page.runtime(), "history.length").await;
    assert!(length > 1);

    arun_async(page.runtime(), "history.back()").await;
    assert_eq!(
        eval_str(page.runtime(), "location.href").await,
        "http://localhost/a"
    );

    arun_async(page.runtime(), "history.forward()").await;
    assert_eq!(
        eval_str(page.runtime(), "location.href").await,
        "http://localhost/b"
    );
}

/// `goto()`'s self-fetched document can itself carry a `type="module" src="...">` script,
/// backed by a resource mount rather than the mock's in-memory routes.
#[tokio::test]
async fn goto_executes_a_module_script_from_a_resource_mount() {
    let dir = std::env::temp_dir().join(format!("miniclient-goto-module-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("entry.js"), b"window.__ran = 1;").unwrap();
    let mounts = format!(
        r#"{{"http://localhost/ext/": {:?}}}"#,
        dir.to_str().unwrap()
    );
    let page = Browser::new(&mounts, false).new_page();
    let mock = MockFetchBackend::new();
    mock.text(
        "http://localhost/",
        "<script type=\"module\" src=\"http://localhost/ext/entry.js\"></script>",
    );
    page.install_fetch_backend(Arc::new(mock)).await;

    page.goto("http://localhost/").await.unwrap();
    let ran: i64 = aeval_async(page.runtime(), "window.__ran").await;
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(ran, 1);
}

/// A hash change queues a timer-driven `hashchange` dispatch; `goto()` right after must not
/// leave the native settle-wait stuck waiting on it.
#[tokio::test]
async fn goto_after_hash_change_does_not_hang() {
    let page = Browser::new("{}", false).new_page();
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

/// A page nothing has navigated yet reuses its blank realm for the first `goto`; any later
/// navigation boots a fresh isolate, so the marker survives exactly one `goto`.
#[tokio::test]
async fn only_the_first_goto_keeps_the_realm() {
    let page = Browser::new("{}", false).new_page();
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/a", "<p id=\"a\">a</p>");
    mock.text("http://localhost/b", "<p id=\"b\">b</p>");
    page.install_fetch_backend(Arc::new(mock)).await;
    arun_async(page.runtime(), "globalThis.__marker = 1").await;

    page.goto("http://localhost/a").await.unwrap();
    assert_eq!(
        eval_str(page.runtime(), "location.href").await,
        "http://localhost/a"
    );
    let kept: i64 = aeval_async(page.runtime(), "globalThis.__marker").await;
    assert_eq!(kept, 1);
    assert!(page.find("#a", None).await.unwrap().is_some());

    page.goto("http://localhost/b").await.unwrap();
    let gone = eval_str(page.runtime(), "typeof globalThis.__marker").await;
    assert_eq!(gone, "undefined");
    assert!(page.find("#b", None).await.unwrap().is_some());
}

/// `load` puts content in the blank document, so the next `goto` is no longer a first
/// navigation and must isolate it.
#[tokio::test]
async fn goto_after_load_boots_a_fresh_isolate() {
    let page = Browser::new("{}", false).new_page();
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/a", "<p id=\"a\">a</p>");
    page.install_fetch_backend(Arc::new(mock)).await;
    arun_async(page.runtime(), "globalThis.__marker = 1").await;
    page.load("<p id=\"loaded\">x</p>").await.unwrap();

    page.goto("http://localhost/a").await.unwrap();
    let gone = eval_str(page.runtime(), "typeof globalThis.__marker").await;
    assert_eq!(gone, "undefined");
    assert!(page.find("#a", None).await.unwrap().is_some());
}

/// The first navigation replaces the blank entry, like a browser's initial document.
#[tokio::test]
async fn first_goto_replaces_the_blank_history_entry() {
    let page = Browser::new("{}", false).new_page();
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/a", "<p>a</p>");
    page.install_fetch_backend(Arc::new(mock)).await;

    page.goto("http://localhost/a").await.unwrap();
    let length: i64 = aeval_async(page.runtime(), "history.length").await;
    assert_eq!(length, 1);
}

#[tokio::test]
async fn first_goto_commits_an_error_document_when_the_fetch_fails() {
    let page = Browser::new("{}", false).new_page();
    let mock = MockFetchBackend::new();
    mock.status("http://localhost/boom", 500);
    page.install_fetch_backend(Arc::new(mock)).await;

    page.goto("http://localhost/boom").await.unwrap();
    assert_eq!(
        eval_str(page.runtime(), "location.href").await,
        "http://localhost/boom"
    );
    assert!(
        page.find("[data-mini-navigation-error]", None)
            .await
            .unwrap()
            .is_some()
    );
}

/// A cookie seeded through `eval` before the first `goto` reaches that document's fetches.
#[tokio::test]
async fn cookie_set_before_the_first_goto_survives_it() {
    let page = Browser::new("{}", false).new_page();
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/a", "<p>a</p>");
    page.install_fetch_backend(Arc::new(mock)).await;
    arun_async(page.runtime(), "document.cookie = 'sid=abc'").await;

    page.goto("http://localhost/a").await.unwrap();
    assert_eq!(eval_str(page.runtime(), "document.cookie").await, "sid=abc");
}
