//! The browser rules `fetch()` adds on top of one HTTP hop: request modes, CORS, redirects,
//! credentials, forbidden headers, body conversion. The page is at `http://localhost/`.

mod common;

use common::EvalExt;
use common::fetch_mock::{MockFetchBackend, runtime_with_mock};
use serde_json::{Value, json};

const SAME: &str = "http://localhost";
const OTHER: &str = "http://api.example.com";

/// What `fetch(url, init)` settles to: the response summary, or `rejected:<error name>`.
fn outcome(mock: &MockFetchBackend, url: &str, init: &str) -> Value {
    let rt = runtime_with_mock(mock);
    rt.eval_promise::<Value>(&format!(
        "fetch('{url}', {init}).then(
           r => ({{type: r.type, status: r.status, url: r.url, redirected: r.redirected}}),
           e => 'rejected:' + e.name,
         )"
    ))
}

#[test]
fn relative_urls_resolve_against_the_page() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/data", "here");
    let rt = runtime_with_mock(&mock);
    assert_eq!(
        rt.eval_promise::<String>("fetch('/data').then(r => r.text())"),
        "here"
    );
}

#[test]
fn same_origin_response_is_basic_and_cross_origin_is_cors() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/a", "a");
    mock.text("http://api.example.com/b", "b");
    assert_eq!(outcome(&mock, "/a", "{}")["type"], "basic");
    assert_eq!(outcome(&mock, &format!("{OTHER}/b"), "{}")["type"], "cors");
}

#[test]
fn redirect_follow_reports_the_chain() {
    let mock = MockFetchBackend::new();
    mock.redirect("http://localhost/old", "/new");
    mock.text("http://localhost/new", "moved");
    let seen = outcome(&mock, "/old", "{}");
    assert_eq!(seen["url"], "http://localhost/new");
    assert_eq!(seen["redirected"], true);
    assert_eq!(mock.requests().len(), 2, "one backend call per hop");
}

#[test]
fn redirect_manual_yields_an_opaque_redirect() {
    let mock = MockFetchBackend::new();
    mock.redirect("http://localhost/old", "/new");
    mock.text("http://localhost/new", "moved");
    let seen = outcome(&mock, "/old", "{redirect: 'manual'}");
    assert_eq!(seen["type"], "opaqueredirect");
    assert_eq!(seen["status"], 0);
    assert_eq!(seen["url"], "http://localhost/old");
    assert_eq!(mock.requests().len(), 1, "the redirect is not followed");
}

#[test]
fn redirect_error_rejects() {
    let mock = MockFetchBackend::new();
    mock.redirect("http://localhost/old", "/new");
    mock.text("http://localhost/new", "moved");
    assert_eq!(
        outcome(&mock, "/old", "{redirect: 'error'}"),
        "rejected:TypeError"
    );
}

#[test]
fn non_redirect_3xx_with_location_is_returned_under_error_and_follow() {
    for status in [300, 304] {
        for mode in ["error", "follow"] {
            let mock = MockFetchBackend::new();
            mock.reply("http://localhost/x", status, &[("location", "/new")], b"");
            mock.text("http://localhost/new", "moved");
            let seen = outcome(&mock, "/x", &format!("{{redirect: '{mode}'}}"));
            assert_eq!(
                (&seen["type"], &seen["status"]),
                (&json!("basic"), &json!(status)),
                "{status} under {mode}"
            );
            assert_eq!(
                mock.requests().len(),
                1,
                "{status} under {mode} is not followed"
            );
        }
    }
}

#[test]
fn redirect_status_without_location_follows_the_redirect_mode() {
    let mock = MockFetchBackend::new();
    mock.status("http://localhost/x", 302);
    assert_eq!(
        outcome(&mock, "/x", "{redirect: 'error'}"),
        "rejected:TypeError"
    );
    let seen = outcome(&mock, "/x", "{redirect: 'manual'}");
    assert_eq!(
        (&seen["type"], &seen["status"]),
        (&json!("opaqueredirect"), &json!(0))
    );
    let seen = outcome(&mock, "/x", "{redirect: 'follow'}");
    assert_eq!(
        (&seen["type"], &seen["status"]),
        (&json!("basic"), &json!(302))
    );
}

#[test]
fn a_303_after_post_becomes_a_get_without_body() {
    let mock = MockFetchBackend::new();
    mock.reply("http://localhost/form", 303, &[("location", "/done")], b"");
    mock.text("http://localhost/done", "ok");
    outcome(&mock, "/form", "{method: 'POST', body: 'x=1'}");
    let last = mock.last_request();
    assert_eq!((last.method.as_str(), last.body.len()), ("GET", 0));
}

#[test]
fn no_cors_cross_origin_is_opaque() {
    let mock = MockFetchBackend::new();
    mock.reply(
        &format!("{OTHER}/b"),
        200,
        &[("access-control-allow-origin", "https://x.test")],
        b"b",
    );
    let seen = outcome(&mock, &format!("{OTHER}/b"), "{mode: 'no-cors'}");
    assert_eq!(
        (&seen["type"], &seen["status"], &seen["url"]),
        (&json!("opaque"), &json!(0), &json!(""))
    );
}

#[test]
fn same_origin_mode_rejects_cross_origin_requests() {
    let mock = MockFetchBackend::new();
    mock.text(&format!("{OTHER}/b"), "b");
    assert_eq!(
        outcome(&mock, &format!("{OTHER}/b"), "{mode: 'same-origin'}"),
        "rejected:TypeError"
    );
    assert!(mock.requests().is_empty(), "nothing reaches the wire");
}

#[test]
fn cors_response_allowing_another_origin_rejects() {
    let mock = MockFetchBackend::new();
    mock.reply(
        &format!("{OTHER}/b"),
        200,
        &[("access-control-allow-origin", "https://x.test")],
        b"b",
    );
    assert_eq!(
        outcome(&mock, &format!("{OTHER}/b"), "{}"),
        "rejected:TypeError"
    );
}

#[test]
fn cors_responses_expose_only_safelisted_and_listed_headers() {
    let mock = MockFetchBackend::new();
    mock.reply(
        &format!("{OTHER}/b"),
        200,
        &[
            ("content-type", "text/plain"),
            ("x-secret", "1"),
            ("x-open", "2"),
            ("access-control-expose-headers", "x-open"),
        ],
        b"b",
    );
    let rt = runtime_with_mock(&mock);
    assert_eq!(
        rt.eval_promise::<Value>(&format!(
            "fetch('{OTHER}/b').then(r => [r.headers.get('content-type'), r.headers.get('x-secret'), r.headers.get('x-open')])"
        )),
        json!(["text/plain", null, "2"])
    );
}

#[test]
fn origin_is_sent_on_cross_origin_and_on_same_origin_writes_only() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/a", "a");
    mock.text("http://api.example.com/b", "b");
    outcome(&mock, "/a", "{}");
    assert_eq!(mock.last_request().header("origin"), None);
    outcome(&mock, "/a", "{method: 'POST', body: 'x'}");
    assert_eq!(mock.last_request().header("origin"), Some(SAME));
    outcome(&mock, &format!("{OTHER}/b"), "{}");
    assert_eq!(mock.last_request().header("origin"), Some(SAME));
}

#[test]
fn referer_is_the_page_unless_the_request_drops_it() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/a", "a");
    outcome(&mock, "/a", "{}");
    assert_eq!(
        mock.last_request().header("referer"),
        Some("http://localhost/")
    );
    outcome(&mock, "/a", "{referrer: ''}");
    assert_eq!(mock.last_request().header("referer"), None);
}

#[test]
fn forbidden_request_headers_are_dropped() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/a", "a");
    outcome(
        &mock,
        "/a",
        "{headers: {Cookie: 'a=b', Origin: 'http://evil', 'Sec-Foo': '1', 'X-Ok': '1'}}",
    );
    let last = mock.last_request();
    assert_eq!(
        (
            last.header("cookie"),
            last.header("origin"),
            last.header("sec-foo"),
            last.header("x-ok")
        ),
        (None, None, None, Some("1"))
    );
}

#[test]
fn a_custom_header_cross_origin_is_preflighted() {
    let mock = MockFetchBackend::new();
    mock.text("http://api.example.com/b", "b");
    outcome(
        &mock,
        "http://api.example.com/b",
        "{headers: {'X-Custom': '1'}}",
    );
    let methods: Vec<_> = mock.requests().into_iter().map(|r| r.method).collect();
    assert_eq!(methods, ["OPTIONS", "GET"]);
    let preflight = &mock.requests()[0];
    assert_eq!(
        preflight.header("access-control-request-headers"),
        Some("x-custom")
    );
}

#[test]
fn credentials_decide_whether_cookies_travel() {
    let rt_cookie = |init: &str, url: &str| {
        let mock = MockFetchBackend::new();
        mock.reply(
            url,
            200,
            &[
                ("set-cookie", "s=1; Path=/"),
                ("access-control-allow-origin", "http://localhost"),
                ("access-control-allow-credentials", "true"),
            ],
            b"",
        );
        let rt = runtime_with_mock(&mock);
        rt.run("document.cookie = 'c=1; path=/'");
        let stored = rt.eval_promise::<String>(&format!(
            "fetch('{url}', {init}).then(() => document.cookie)"
        ));
        (
            mock.last_request().header("cookie").map(str::to_owned),
            stored,
        )
    };
    let url = "http://localhost/a";
    assert_eq!(
        rt_cookie("{}", url),
        (Some("c=1".into()), "c=1; s=1".into())
    );
    assert_eq!(
        rt_cookie("{credentials: 'omit'}", url),
        (None, "c=1".into())
    );
}

fn same_origin_credentials_chain(via: &str) -> (Vec<Option<String>>, String) {
    let mock = MockFetchBackend::new();
    let cors = [("access-control-allow-origin", "*")];
    mock.redirect("http://localhost/a", via);
    mock.reply(
        &format!("{OTHER}/b"),
        302,
        &[cors[0], ("location", "http://localhost/c")],
        b"",
    );
    mock.reply(
        "http://localhost/c",
        200,
        &[cors[0], ("set-cookie", "s=1; Path=/")],
        b"",
    );
    mock.reply("http://localhost/mid", 302, &[("location", "/c")], b"");
    let rt = runtime_with_mock(&mock);
    rt.run("document.cookie = 'c=1; path=/'");
    let stored = rt.eval_promise::<String>(
        "fetch('/a', {credentials: 'same-origin'}).then(() => document.cookie)",
    );
    let sent = mock
        .requests()
        .iter()
        .map(|r| r.header("cookie").map(str::to_owned))
        .collect();
    (sent, stored)
}

#[test]
fn same_origin_credentials_stop_once_the_chain_left_the_origin() {
    let (sent, stored) = same_origin_credentials_chain(&format!("{OTHER}/b"));
    assert_eq!(sent, [Some("c=1".into()), None, None]);
    assert_eq!(stored, "c=1");
}

#[test]
fn same_origin_credentials_survive_a_same_origin_chain() {
    let (sent, stored) = same_origin_credentials_chain("/mid");
    assert_eq!(sent, vec![Some("c=1".to_string()); 3]);
    assert_eq!(stored, "c=1; s=1");
}

const ALLOW_PAGE: [(&str, &str); 2] = [
    ("access-control-allow-origin", "http://localhost"),
    ("access-control-allow-credentials", "true"),
];
const ALLOW_ANY: [(&str, &str); 2] = [
    ("access-control-allow-origin", "*"),
    ("access-control-allow-credentials", "true"),
];
const INCLUDE: &str = "{credentials: 'include'}";

fn passes(seen: &Value) -> bool {
    seen["type"] == "cors"
}

#[test]
fn credentialed_final_response_needs_an_exact_origin() {
    let url = format!("{OTHER}/b");
    let mock = MockFetchBackend::new();
    mock.reply(&url, 200, &ALLOW_ANY, b"");
    assert_eq!(outcome(&mock, &url, INCLUDE), "rejected:TypeError");
    assert!(
        passes(&outcome(&mock, &url, "{}")),
        "uncredentialed accepts *"
    );
    mock.reply(&url, 200, &ALLOW_PAGE, b"");
    assert!(passes(&outcome(&mock, &url, INCLUDE)));
}

#[test]
fn credentialed_redirect_hop_needs_an_exact_origin() {
    let hop = format!("{OTHER}/a");
    let mock = MockFetchBackend::new();
    // The origin turns opaque ("null") after a cross-origin to cross-origin redirect.
    let opaque = [ALLOW_PAGE[1], ("access-control-allow-origin", "null")];
    mock.reply("http://third.example.com/c", 200, &opaque, b"");
    let location = ("location", "http://third.example.com/c");
    mock.reply(&hop, 302, &[ALLOW_ANY[0], ALLOW_ANY[1], location], b"");
    assert_eq!(outcome(&mock, &hop, INCLUDE), "rejected:TypeError");
    mock.reply(&hop, 302, &[ALLOW_PAGE[0], ALLOW_PAGE[1], location], b"");
    assert!(passes(&outcome(&mock, &hop, INCLUDE)));
}

#[test]
fn credentialed_preflight_response_needs_an_exact_origin() {
    let url = format!("{OTHER}/b");
    let init = "{credentials: 'include', headers: {'X-Custom': '1'}}";
    let mock = MockFetchBackend::new();
    mock.reply(&url, 200, &ALLOW_PAGE, b"");
    mock.preflight(
        &url,
        &[
            ALLOW_ANY[0],
            ALLOW_ANY[1],
            ("access-control-allow-headers", "x-custom"),
        ],
    );
    assert_eq!(outcome(&mock, &url, init), "rejected:TypeError");
    mock.preflight(
        &url,
        &[
            ALLOW_PAGE[0],
            ALLOW_PAGE[1],
            ("access-control-allow-headers", "x-custom"),
        ],
    );
    assert!(passes(&outcome(&mock, &url, init)));
}

#[test]
fn a_preflighted_method_must_be_allowed() {
    let url = format!("{OTHER}/b");
    let mock = MockFetchBackend::new();
    mock.reply(&url, 200, &[], b"");
    let put = |mock: &MockFetchBackend, init: &str| outcome(mock, &url, init);
    for (allow, init, ok) in [
        ("GET, POST", "{method: 'PUT'}", false),
        ("GET, PUT", "{method: 'PUT'}", true),
        ("*", "{method: 'PUT'}", true),
        ("*", "{method: 'PUT', credentials: 'include'}", false),
    ] {
        let headers = [("access-control-allow-methods", allow)];
        if init.contains("include") {
            mock.preflight(&url, &[headers[0], ALLOW_PAGE[0], ALLOW_PAGE[1]]);
            mock.reply(&url, 200, &ALLOW_PAGE, b"");
        } else {
            mock.preflight(&url, &headers);
        }
        assert_eq!(passes(&put(&mock, init)), ok, "{allow} for {init}");
    }
}

#[test]
fn credentialed_allow_headers_wildcard_covers_nothing() {
    let url = format!("{OTHER}/b");
    let mock = MockFetchBackend::new();
    mock.reply(&url, 200, &ALLOW_PAGE, b"");
    let init = "{credentials: 'include', headers: {'X-Custom': '1'}}";
    let preflight = |allow: &'static str| {
        mock.preflight(
            &url,
            &[
                ALLOW_PAGE[0],
                ALLOW_PAGE[1],
                ("access-control-allow-headers", allow),
            ],
        );
    };
    preflight("*");
    assert_eq!(outcome(&mock, &url, init), "rejected:TypeError");
    preflight("x-custom");
    assert!(passes(&outcome(&mock, &url, init)));
}

#[test]
fn uncredentialed_allow_headers_wildcard_never_covers_authorization() {
    let url = format!("{OTHER}/b");
    let mock = MockFetchBackend::new();
    mock.reply(&url, 200, &[], b"");
    mock.preflight(&url, &[("access-control-allow-headers", "*")]);
    let custom = outcome(&mock, &url, "{headers: {'X-Custom': '1'}}");
    assert!(passes(&custom));
    let auth = outcome(&mock, &url, "{headers: {'Authorization': 'x'}}");
    assert_eq!(auth, "rejected:TypeError");
}

#[test]
fn blob_file_and_form_data_bodies_keep_their_bytes() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/up", "ok");
    let rt = runtime_with_mock(&mock);
    rt.eval_promise::<String>(
        "fetch('/up', {method: 'POST', body: new Blob(['abc'], {type: 'text/x-t'})}).then(r => r.text())",
    );
    let blob = mock.last_request();
    assert_eq!(
        (blob.body.as_slice(), blob.header("content-type")),
        (&b"abc"[..], Some("text/x-t"))
    );

    rt.eval_promise::<String>(
        "(() => { const f = new FormData(); f.append('k', 'v');
           f.append('up', new File(['bytes'], 'f.txt', {type: 'text/plain'}));
           return fetch('/up', {method: 'POST', body: f}).then(r => r.text()); })()",
    );
    let form = mock.last_request();
    let text = String::from_utf8_lossy(&form.body);
    assert!(
        form.header("content-type")
            .unwrap()
            .starts_with("multipart/form-data; boundary=")
    );
    assert!(
        text.contains("name=\"k\"")
            && text.contains("filename=\"f.txt\"")
            && text.contains("bytes"),
        "{text}"
    );
}

#[test]
fn abort_between_redirect_hops_rejects() {
    let mock = MockFetchBackend::new();
    mock.redirect("http://localhost/old", "/slow");
    mock.hang("http://localhost/slow");
    let rt = runtime_with_mock(&mock);
    let name = rt.eval_promise::<String>(
        "(async () => {
           const c = new AbortController();
           const p = fetch('/old', {signal: c.signal}).then(() => 'resolved', e => e.name);
           await new Promise(r => setTimeout(r, 20));
           c.abort();
           return await p;
         })()",
    );
    assert_eq!(name, "AbortError");
    assert_eq!(
        mock.aborted().len(),
        1,
        "the second hop was cancelled on the wire"
    );
}

#[test]
fn same_origin_mode_rejects_a_cross_origin_redirect_before_sending_it() {
    let mock = MockFetchBackend::new();
    mock.redirect("http://localhost/old", &format!("{OTHER}/b"));
    mock.text(&format!("{OTHER}/b"), "b");
    assert_eq!(
        outcome(&mock, "/old", "{mode: 'same-origin'}"),
        "rejected:TypeError"
    );
    assert!(
        mock.requests().iter().all(|r| !r.url.starts_with(OTHER)),
        "the other origin never sees the request"
    );
}

#[test]
fn same_origin_mode_follows_a_same_origin_redirect_chain() {
    let mock = MockFetchBackend::new();
    mock.redirect("http://localhost/old", "/mid");
    mock.redirect("http://localhost/mid", "/new");
    mock.text("http://localhost/new", "moved");
    let seen = outcome(&mock, "/old", "{mode: 'same-origin'}");
    assert_eq!(
        (&seen["type"], &seen["redirected"]),
        (&json!("basic"), &json!(true))
    );
}
