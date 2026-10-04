//! `browser::Page`/`browser::Element` driving real htmx behavior end to end (goto/load, click,
//! custom triggers, form submission), backed by the vendored `htmx.js` served from a resource
//! mount and `MockFetchBackend` -- no pyo3/Python involved.

mod common;

use std::sync::Arc;

use _miniclient::browser::Page;
use common::fetch_mock::MockFetchBackend;

const VENDOR_HTMX_SRC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/htmx/src");
const HTMX_SCRIPT_TAG: &str = "<script src='http://localhost/vendor/htmx.js'></script>";

fn htmx_mounts() -> String {
    format!(r#"{{"http://localhost/vendor/": {VENDOR_HTMX_SRC:?}}}"#)
}

/// A fresh `Page` that can reach the vendored `htmx.js` via a resource mount, with the given
/// mock fetch backend installed.
async fn htmx_page(mock: &MockFetchBackend) -> Page {
    let page = Page::new(&htmx_mounts());
    page.install_fetch_backend(Arc::new(mock.clone())).await;
    page
}

#[tokio::test]
async fn goto_processes_htmx() {
    let mock = MockFetchBackend::new();
    mock.text(
        "http://localhost/page",
        &format!(
            "<html><head>{HTMX_SCRIPT_TAG}</head><body>\
             <div id='out'><button hx-get='/frag' hx-target='#out' \
             hx-swap='innerHTML'>go</button></div></body></html>"
        ),
    );
    mock.text("http://localhost/frag", "<b>done</b>");
    let page = htmx_page(&mock).await;

    page.goto("http://localhost/page").await.unwrap();
    let btn = page.find("button", None).await.unwrap().unwrap();
    btn.click().await.unwrap();
    let out = page.find("#out", None).await.unwrap().unwrap();
    assert_eq!(out.inner_html().await.unwrap(), "<b>done</b>");
}

#[tokio::test]
async fn element_click_triggers_hx_get() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/click-target", "<b>clicked</b>");
    let page = htmx_page(&mock).await;
    page.load(&format!(
        "{HTMX_SCRIPT_TAG}\
         <div id='out'><button hx-get='/click-target' hx-target='#out' \
         hx-swap='innerHTML'>click</button></div>"
    ))
    .await
    .unwrap();

    let btn = page.find("button", None).await.unwrap().unwrap();
    btn.click().await.unwrap();
    let out = page.find("#out", None).await.unwrap().unwrap();
    assert_eq!(out.inner_html().await.unwrap(), "<b>clicked</b>");
}

#[tokio::test]
async fn element_trigger_fires_a_custom_hx_trigger_event() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/custom", "<i>custom</i>");
    let page = htmx_page(&mock).await;
    page.load(&format!(
        "{HTMX_SCRIPT_TAG}\
         <div id='out'><button hx-get='/custom' hx-trigger='my-event' \
         hx-target='#out' hx-swap='innerHTML'>go</button></div>"
    ))
    .await
    .unwrap();

    let btn = page.find("button", None).await.unwrap().unwrap();
    btn.trigger("my-event", None).await.unwrap();
    let out = page.find("#out", None).await.unwrap().unwrap();
    assert_eq!(out.inner_html().await.unwrap(), "<i>custom</i>");
}

#[tokio::test]
async fn request_submit_triggers_an_hx_post_form() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/form-action", "<p>submitted</p>");
    let page = htmx_page(&mock).await;
    page.load(&format!(
        "{HTMX_SCRIPT_TAG}\
         <form id='f' hx-post='/form-action' hx-target='#result' hx-swap='innerHTML'>\
         <input name='x' value='1'>\
         <button type='submit' id='btn'>send</button></form>\
         <div id='result'></div>"
    ))
    .await
    .unwrap();

    let form = page.find("form", None).await.unwrap().unwrap();
    assert_eq!(form.tag_name().await.unwrap(), "FORM");
    form.request_submit().await.unwrap();
    let result = page.find("#result", None).await.unwrap().unwrap();
    assert_eq!(result.inner_html().await.unwrap(), "<p>submitted</p>");
}

async fn assert_submit_via_submitter_click(submitter_html: &str) {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/form-action", "<p>sent</p>");
    let page = htmx_page(&mock).await;
    page.load(&format!(
        "{HTMX_SCRIPT_TAG}\
         <form hx-post='/form-action' hx-target='#result' hx-swap='innerHTML'>\
         {submitter_html}</form><div id='result'></div>"
    ))
    .await
    .unwrap();

    let sub = page.find("#sub", None).await.unwrap().unwrap();
    sub.click().await.unwrap();
    let result = page.find("#result", None).await.unwrap().unwrap();
    assert_eq!(result.inner_html().await.unwrap(), "<p>sent</p>");
}

#[tokio::test]
async fn submit_via_input_submitter_click() {
    assert_submit_via_submitter_click(
        r#"<input type="text" name="q" value="search">
           <input type="submit" id="sub" value="go">"#,
    )
    .await;
}

#[tokio::test]
async fn submit_via_button_submitter_click() {
    assert_submit_via_submitter_click(
        r#"<input type="text" name="q" value="search">
           <button type="submit" id="sub">go</button>"#,
    )
    .await;
}

/// A live `hx-trigger="every ..."` poller elsewhere on the page must not make `click()` on an
/// unrelated element hang: `bootstrap.js` deliberately does not track `setInterval` in the
/// `AsyncTaskManager` reconnection (jsdom's native `Window.setInterval` starts a timer with no
/// matching `endTimer()` until `clearInterval()`), so a live poller would otherwise keep
/// `waitUntilComplete()` permanently pending.
#[tokio::test]
async fn click_is_not_blocked_by_an_unrelated_poller() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/poll", "<span id='poller'>tick</span>");
    mock.text("http://localhost/click-target", "<b>clicked</b>");
    let page = htmx_page(&mock).await;
    page.load(&format!(
        "{HTMX_SCRIPT_TAG}\
         <span id='poller' hx-get='/poll' hx-trigger='every 50ms' \
         hx-swap='outerHTML'></span>\
         <div id='out'><button hx-get='/click-target' hx-target='#out' \
         hx-swap='innerHTML'>click</button></div>"
    ))
    .await
    .unwrap();

    let btn = page.find("button", None).await.unwrap().unwrap();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), btn.click()).await;
    outcome
        .expect("click() hung instead of settling")
        .expect("click() must not error");
    let out = page.find("#out", None).await.unwrap().unwrap();
    assert_eq!(out.inner_html().await.unwrap(), "<b>clicked</b>");
}
