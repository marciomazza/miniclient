//! The blank `Window` is built once into the snapshot and every runtime boots from it: nothing a
//! page did in one runtime may show in the next, and clocks it read at build time are reset.

mod common;

use std::sync::Arc;

use _miniclient::browser::Browser;
use common::fetch_mock::MockFetchBackend;
use common::{EvalExt, aeval_async, arun_async, runtime};

#[test]
fn a_fresh_runtime_sees_none_of_another_runtimes_state() {
    let first = runtime();
    first.run(
        r#"localStorage.setItem("k", "v"); document.cookie = "z=9; path=/";
        window.leak = 1; document.title = "t"; history.pushState({ a: 1 }, "", "/q");
        document.head.innerHTML = "<style>p { color: red }</style>";
        document.body.innerHTML = "<p>dirty</p>"; 0"#,
    );
    let second = runtime();
    let got: Vec<String> = second.eval(
        r#"[String(localStorage.length), document.cookie, typeof window.leak, document.title,
        location.href, String(history.length), String(history.state), document.referrer,
        String(document.styleSheets.length), document.body.innerHTML, document.head.innerHTML]"#,
    );
    assert_eq!(
        got,
        [
            "0",
            "",
            "undefined",
            "",
            "http://localhost/",
            "1",
            "null",
            "",
            "0",
            "",
            ""
        ]
    );
}

#[test]
fn clocks_read_at_snapshot_build_are_reset_for_each_runtime() {
    let rt = runtime();
    let age_s: f64 = rt.eval(
        r#"const [m, d, y, h, mi, s] = document.lastModified.match(/\d+/g).map(Number);
        Math.abs(Date.now() - new Date(y, m - 1, d, h, mi, s).getTime()) / 1000"#,
    );
    assert!(age_s < 60.0, "document.lastModified is {age_s}s off");
    // jsdom's own Performance counts from this runtime's start; the animation-frame timestamp
    // is relative to the same origin, so it can never run ahead of `now()`.
    let got: Vec<f64> = rt.eval_promise(
        r#"new Promise((resolve) => requestAnimationFrame((t) => {
            const perf = __jsdomImplForWrapper(document.defaultView.performance);
            resolve([perf.now(), t, perf.now()]);
        }))"#,
    );
    assert!((0.0..5000.0).contains(&got[0]), "now() is {}", got[0]);
    assert!(
        got[1] <= got[2] + 50.0,
        "frame timestamp {} past now() {}",
        got[1],
        got[2]
    );
}

/// A JS-triggered navigation to the blank URL boots from the baked window with host state: the
/// carried cookies must fill the baked jar (and reach requests) and the history must be seeded.
#[tokio::test]
async fn a_navigation_boot_at_the_blank_url_keeps_host_cookies_and_history() {
    let page = Browser::new("{}", false).new_page();
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/a", "<p>a</p>");
    mock.text("http://localhost/", "<p>blank</p>");
    mock.text("http://localhost/ping", "ok");
    page.install_fetch_backend(Arc::new(mock.clone())).await;
    page.goto("http://localhost/a").await.unwrap();
    arun_async(&page.runtime(), "document.cookie = 'c=1; path=/'").await;
    arun_async(
        &page.runtime(),
        "Deno.core.ops.op_request_navigation('http://localhost/', 'push')",
    )
    .await;
    let got: Vec<String> = aeval_async(
        &page.runtime(),
        r#"fetch("/ping").then(() =>
        [location.href, document.cookie, String(history.length), document.body.textContent])"#,
    )
    .await;
    assert_eq!(got, ["http://localhost/", "c=1", "2", "blank"]);
    let cookie = mock.last_request().header("cookie").map(str::to_string);
    assert_eq!(cookie.as_deref(), Some("c=1"));
}
