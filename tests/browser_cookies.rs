//! Cookie handling over `fetch()` (`bootstrap.js`'s `cookieJar`, see `js/bootstrap.js`), driven
//! through `browser::Page`/`MockFetchBackend` -- no real WSGI app, httpx, or pyo3 involved.
//! The WSGI-specific case only exercised fetch-backend-agnostic `goto()`/`find()` plumbing,
//! already covered by `tests/browser_navigation.rs`'s `goto_loads_head_and_body` et al.

mod common;

use std::sync::Arc;

use _miniclient::browser::Browser;
use common::aeval_async;
use common::fetch_mock::MockFetchBackend;

/// `document.cookie` set on one page must reach the `Cookie` request header of a subsequent
/// same-origin fetch (here, the next `goto()`'s own navigation fetch).
#[tokio::test]
async fn document_cookie_reaches_request_cookie_header() {
    let page = Browser::new("{}", false).new_page();
    let mock = MockFetchBackend::new();
    mock.text("http://testserver/", "<p id='cookie'></p>");
    page.install_fetch_backend(Arc::new(mock.clone())).await;

    page.goto("http://testserver/").await.unwrap();
    common::arun_async(
        &page.runtime(),
        "document.cookie = 'sessionid=abc123; path=/'",
    )
    .await;

    page.goto("http://testserver/").await.unwrap();
    let cookie_header = mock
        .last_request()
        .header("cookie")
        .expect("cookie header must be sent")
        .to_string();
    assert!(cookie_header.contains("sessionid=abc123"));
}

/// A `Set-Cookie` response header reaches `document.cookie` -- except an `HttpOnly` one, which
/// must still land in the jar (and so still reach the next request) but stay hidden from
/// script.
#[tokio::test]
async fn set_cookie_response_reaches_document_cookie_unless_http_only() {
    for (set_cookie_header, expected_document_cookie) in [
        ("sessionid=xyz789; Path=/", "sessionid=xyz789"),
        ("sessionid=httponlyval; Path=/; HttpOnly", ""),
    ] {
        let page = Browser::new("{}", false).new_page();
        let mock = MockFetchBackend::new();
        mock.reply(
            "http://testserver/login",
            200,
            &[
                ("content-type", "text/html"),
                ("set-cookie", set_cookie_header),
            ],
            b"<p id='cookie'></p>",
        );
        page.install_fetch_backend(Arc::new(mock)).await;

        page.goto("http://testserver/login").await.unwrap();
        let document_cookie: String = aeval_async(&page.runtime(), "document.cookie").await;
        assert_eq!(document_cookie, expected_document_cookie);
    }
}

/// A hop that answers a redirect + `Set-Cookie` (e.g. a login rotating the session) must still
/// land the cookie in jsdom's jar even though it never appears on the final response's headers.
#[tokio::test]
async fn cookie_set_on_a_followed_redirect_reaches_the_jar() {
    let page = Browser::new("{}", false).new_page();
    let mock = MockFetchBackend::new();
    mock.reply(
        "http://testserver/login",
        302,
        &[
            ("location", "http://testserver/echo"),
            ("set-cookie", "sessionid=rotated; Path=/"),
        ],
        b"",
    );
    mock.text("http://testserver/echo", "ok");
    page.install_fetch_backend(Arc::new(mock)).await;

    page.goto("http://testserver/login").await.unwrap();
    let document_cookie: String = aeval_async(&page.runtime(), "document.cookie").await;
    assert_eq!(document_cookie, "sessionid=rotated");
}

// `test_cookie_survives_auto_followed_redirect` stays pytest-only: it asserts that *httpx's
// own* internal redirect follow (`AsyncPage`'s real `httpx.AsyncClient`) reseeds the Cookie
// header on each hop from its jar (see `fetch.py`'s `_do_fetch` comment on
// `_redirect_headers`) -- behavior specific to that concrete `FetchBackend` implementation, not
// to `bootstrap.js`/jsdom or to the generic `FetchBackend` trait `MockFetchBackend` stands in
// for here. `MockFetchBackend` has no redirect-following HTTP client of its own to reseed.

/// Host forms that take the IPv6/IP paths in tough-cookie (literals, loopback, a plain domain)
/// must keep accepting host-only and `Domain` cookies.
#[tokio::test]
async fn document_cookie_works_on_ip_literal_and_domain_hosts() {
    for (url, cookie) in [
        ("http://localhost/", "a=1; Secure"),
        ("http://[::1]/", "a=1"),
        ("http://127.0.0.1/", "a=1"),
        ("http://sub.example.com/", "a=1; Domain=example.com"),
    ] {
        let page = Browser::new("{}", false).new_page();
        let mock = MockFetchBackend::new();
        mock.text(url, "<p></p>");
        page.install_fetch_backend(Arc::new(mock)).await;
        page.goto(url).await.unwrap();
        common::arun_async(&page.runtime(), &format!("document.cookie = '{cookie}'")).await;
        let document_cookie: String = aeval_async(&page.runtime(), "document.cookie").await;
        assert_eq!(document_cookie, "a=1", "{url}");
    }
}
