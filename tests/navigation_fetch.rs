//! The navigation `op_request_navigation` drives (`Runtime::new`'s own command loop, not
//! `Page::navigate`) boots an empty realm and must self-fetch `__BASE_URL__` to have a document
//! at all -- see `js/bootstrap.js`'s `__NAVIGATION_BOOT__` branch. Drives the op directly, same
//! as `tests/navigation_trigger.rs`, with a `MockFetchBackend` installed via
//! `Page::install_fetch_backend` beforehand.

mod common;

use std::sync::Arc;

use _miniclient::runtime::Page;
use common::EvalExt;
use common::fetch_mock::MockFetchBackend;

#[test]
fn navigation_self_fetches_the_new_document_on_success() {
    let mut page = Page::new("{}");
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/next", "<body>hello from next</body>");
    page.install_fetch_backend(Arc::new(mock));

    page.runtime()
        .run("Deno.core.ops.op_request_navigation('http://localhost/next', 'push'); void 0");

    assert_eq!(
        page.runtime().eval::<String>("location.href"),
        "http://localhost/next"
    );
    assert_eq!(
        page.runtime().eval::<String>("document.body.textContent"),
        "hello from next"
    );
}

#[test]
fn navigation_commits_an_error_document_on_a_failed_fetch() {
    let mut page = Page::new("{}");
    let mock = MockFetchBackend::new();
    // No route for /next -- the mock answers it with a connection-style error.
    page.install_fetch_backend(Arc::new(mock));

    page.runtime()
        .run("Deno.core.ops.op_request_navigation('http://localhost/next', 'push'); void 0");

    // Still commits the new realm -- never reverts to the old document, which is already gone.
    assert_eq!(
        page.runtime().eval::<String>("location.href"),
        "http://localhost/next"
    );
    assert!(
        page.runtime()
            .eval::<bool>("document.querySelector('[data-mini-navigation-error]') !== null")
    );
}

/// A completed fetch commits its body whatever the status, as a browser's top-level navigation
/// does -- only a failed fetch gets the error document.
#[test]
fn navigation_commits_the_body_of_a_non_2xx_response() {
    let mut page = Page::new("{}");
    let mock = MockFetchBackend::new();
    mock.reply("http://localhost/next", 301, &[], b"<body>moved</body>");
    page.install_fetch_backend(Arc::new(mock));

    page.runtime()
        .run("Deno.core.ops.op_request_navigation('http://localhost/next', 'push'); void 0");

    assert_eq!(
        page.runtime().eval::<String>("document.body.textContent"),
        "moved"
    );
    assert!(
        !page
            .runtime()
            .eval::<bool>("document.querySelector('[data-mini-navigation-error]') !== null")
    );
}
