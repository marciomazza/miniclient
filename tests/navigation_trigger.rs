//! `op_request_navigation` wired into `eval()`. Drives the op directly (no `bootstrap.js`
//! wiring yet) to prove the trigger mechanism itself: a plain top-level call simulates an
//! external `Page.goto()`-style trigger, an
//! `await`-then-call simulates JS requesting navigation mid-promise-chain. Both must land in a
//! correctly settled new realm without corrupting the calling `eval`'s own reply.

mod common;

use _miniclient::runtime::Wait;
use common::EvalExt;
use common::fetch_mock::{MockFetchBackend, runtime_with_mock};

/// A landed navigation self-fetches `__BASE_URL__` (see `js/bootstrap.js`'s `__NAVIGATION_BOOT__`
/// branch), so every test below that actually lands a navigation needs a route for wherever it
/// navigates to -- an empty body is enough, none of them assert on fetched content.
fn runtime_with_route(url: &str) -> common::Runtime {
    let mock = MockFetchBackend::new();
    mock.text(url, "");
    runtime_with_mock(&mock)
}

#[test]
fn navigation_trigger_navigates_the_realm_from_a_plain_top_level_script() {
    let rt = runtime_with_route("http://localhost/next");
    rt.run("document.title = 'before'; void 0");

    // The triggering eval's own reply is `undefined` -- its script's completion value belonged
    // to the document being thrown away, so it's discarded rather than marshaled.
    let reply =
        rt.eval_json("Deno.core.ops.op_request_navigation('http://localhost/next', 'push'); 42;");
    assert_eq!(reply, None);

    assert_eq!(rt.eval::<String>("location.href"), "http://localhost/next");
    assert_eq!(rt.eval::<String>("document.title"), "");
}

#[test]
fn navigation_trigger_navigates_the_realm_mid_promise_chain() {
    let rt = runtime_with_route("http://localhost/next");
    rt.run("document.title = 'before'; void 0");

    rt.run_async(
        "(async () => { \
           await Promise.resolve();\
           Deno.core.ops.op_request_navigation('http://localhost/next', 'push')\
         })()",
    );

    assert_eq!(rt.eval::<String>("location.href"), "http://localhost/next");
    assert_eq!(rt.eval::<String>("document.title"), "");

    // The isolate thread and its command channel survived the navigation -- not corrupted, still
    // answering further evals normally.
    assert_eq!(rt.eval::<i32>("1 + 1"), 2);
}

#[test]
fn navigation_trigger_from_a_reentrant_eval_defers_until_the_outer_eval_settles() {
    // A request that lands while a *different* eval is still suspended mid-await (the
    // interleaved/reentrant path `run_with_interleaved_commands` exists for -- see its own doc
    // comment) must not navigate immediately: the outer eval further up the Rust call stack is still
    // holding v8 handles into this isolate. It must wait until that outer eval -- a genuine
    // top-level one -- itself settles.
    let mock = MockFetchBackend::new();
    mock.hang("http://localhost/slow");
    mock.text("http://localhost/next", "");
    let rt = runtime_with_mock(&mock);
    rt.run("globalThis.__controller = new AbortController(); void 0");

    // Outer eval: in flight, suspended on a fetch that only settles once aborted below.
    let outer = rt.send_eval(
        "fetch('http://localhost/slow', {signal: __controller.signal}).catch(() => 'aborted')"
            .into(),
        Wait::Promise,
    );

    // Reentrant, while the outer eval above is still in flight: `handle_command` processes this
    // via the interleaved path with `allow_navigate: false`, so it must defer, not navigate.
    rt.run("Deno.core.ops.op_request_navigation('http://localhost/next', 'push'); void 0");
    assert_eq!(rt.eval::<String>("location.href"), "http://localhost/");

    // Let the outer eval settle -- only then does the deferred navigation happen.
    rt.run("__controller.abort()");
    outer
        .blocking_recv()
        .expect("channel")
        .expect("outer eval succeeded");

    assert_eq!(rt.eval::<String>("location.href"), "http://localhost/next");
}

#[test]
fn navigation_trigger_carries_cookies_forward() {
    let rt = runtime_with_route("http://localhost/next");
    rt.run("document.cookie = 'a=1'");

    rt.run("Deno.core.ops.op_request_navigation('http://localhost/next', 'push'); void 0");

    assert_eq!(rt.eval::<String>("document.cookie"), "a=1");
}

#[test]
fn navigation_trigger_push_appends_a_history_entry() {
    let rt = runtime_with_route("http://localhost/next");
    rt.run("history.pushState({n: 1}, '', '/prev')");

    rt.run("Deno.core.ops.op_request_navigation('http://localhost/next', 'push'); void 0");

    assert_eq!(rt.eval::<String>("location.href"), "http://localhost/next");
    // http://localhost/ -> /prev -> /next
    assert_eq!(rt.eval::<i32>("history.length"), 3);
}

#[test]
fn navigation_trigger_replace_keeps_history_length() {
    let rt = runtime_with_route("http://localhost/next");
    rt.run("history.pushState({n: 1}, '', '/prev')");

    rt.run("Deno.core.ops.op_request_navigation('http://localhost/next', 'replace'); void 0");

    assert_eq!(rt.eval::<String>("location.href"), "http://localhost/next");
    assert_eq!(rt.eval::<i32>("history.length"), 2);
}

#[test]
fn navigation_trigger_carries_a_post_body_and_headers_across_the_navigation() {
    // A POST form submission (submit.js) drives the same op, just with method/body/headers
    // set -- the self-fetch in the new realm must reissue them verbatim, not fall back to GET.
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/next", "");
    let rt = runtime_with_mock(&mock);

    rt.run(
        "Deno.core.ops.op_request_navigation(\
           'http://localhost/next', 'push', 'post', 'a=1', \
           {'content-type': 'application/x-www-form-urlencoded'}\
         ); void 0",
    );

    assert_eq!(rt.eval::<String>("location.href"), "http://localhost/next");
    let req = mock.last_request();
    assert_eq!(req.method, "POST");
    assert_eq!(req.body, b"a=1");
    assert_eq!(
        req.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
}

#[test]
fn navigation_trigger_traverse_to_restores_an_earlier_entry() {
    let rt = runtime_with_route("http://localhost/a");
    rt.run("history.pushState({n: 1}, '', '/a')");
    rt.run("history.pushState({n: 2}, '', '/b')");

    rt.run("Deno.core.ops.op_request_navigation('http://localhost/a', {traverseTo: 1}); void 0");

    assert_eq!(rt.eval::<String>("location.href"), "http://localhost/a");
    assert_eq!(rt.eval::<i32>("history.length"), 3);
    assert_eq!(rt.eval::<i32>("history.state.n"), 1);
}
