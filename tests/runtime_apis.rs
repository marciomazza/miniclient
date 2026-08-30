//! The deno_web, happy-dom and node-polyfill globals the default snapshot boots, exercised
//! through `send_eval`.

mod common;

use common::{EvalExt, boolean, json, runtime, text};

#[test]
fn window_and_document_basics() {
    let rt = runtime();
    assert_eq!(text(rt.eval("typeof window")), "object");
    assert_eq!(
        text(rt.eval("document.createElement('div').tagName")),
        "DIV"
    );
    assert!(!boolean(rt.eval("new AbortController().signal.aborted")));
}

#[test]
fn deno_web_globals_wired() {
    let rt = runtime();
    for name in [
        "URL",
        "URLSearchParams",
        "TextEncoder",
        "TextDecoder",
        "atob",
        "btoa",
        "performance",
        "PerformanceObserver",
        "ReadableStream",
    ] {
        assert!(
            boolean(rt.eval(&format!("typeof globalThis.{name} !== 'undefined'"))),
            "{name}",
        );
    }
}

#[test]
fn buffer() {
    let rt = runtime();
    for (js, want) in [
        ("Buffer.from('hello', 'utf8').toString('utf8')", "hello"),
        ("Buffer.from('hello').toString('base64')", "aGVsbG8="),
        (
            "Buffer.from('aGVsbG8=', 'base64').toString('utf8')",
            "hello",
        ),
        ("Buffer.from('ab', 'utf8').toString('hex')", "6162"),
        ("Buffer.from('6162', 'hex').toString('utf8')", "ab"),
        (
            "Buffer.concat([Buffer.from('foo'), Buffer.from('bar')]).toString()",
            "foobar",
        ),
        ("Buffer.alloc(3).toString('hex')", "000000"),
        ("Buffer.from([0x68, 0x69]).toString()", "hi"),
    ] {
        assert_eq!(text(rt.eval(js)), want, "{js}");
    }
    assert!(boolean(rt.eval("Buffer.isBuffer(Buffer.alloc(4))")));
    assert!(!boolean(rt.eval("Buffer.isBuffer(new Uint8Array(4))")));
}

#[test]
fn dom_manipulation() {
    let rt = runtime();
    rt.run(r#"document.body.innerHTML = '<div id="x"><span class="y">hi</span></div>'"#);
    assert_eq!(
        text(rt.eval("document.querySelector('#x .y').textContent")),
        "hi",
    );

    rt.run("document.body.innerHTML = '<ul><li>a</li><li>b</li><li>c</li></ul>'");
    assert_eq!(
        json(rt.eval("document.querySelectorAll('li').length")).as_deref(),
        Some("3"),
    );

    rt.run(r#"document.body.innerHTML = '<p id="p1">text</p>'"#);
    assert_eq!(
        text(rt.eval("document.getElementById('p1').innerHTML")),
        "text",
    );

    let href =
        text(rt.eval("const a = document.createElement('a'); a.href = 'http://z.com'; a.href"));
    assert!(href.contains("z.com"), "{href:?}");
}

#[test]
fn happy_dom_globals_are_functions() {
    let rt = runtime();
    for js in [
        "typeof Headers",
        "typeof Request",
        "typeof Response",
        "typeof FormData",
        "typeof MutationObserver",
        "typeof CustomEvent",
        "typeof AbortController",
        // EventTarget and DOMParser are on window but not promoted to globalThis.
        "typeof window.EventTarget",
        "typeof window.DOMParser",
    ] {
        assert_eq!(text(rt.eval(js)), "function", "{js}");
    }
}

#[test]
fn headers_dom_parser_and_form_data() {
    let rt = runtime();
    assert_eq!(
        text(rt.eval(
            r#"const h = new Headers({'content-type': 'text/html'}); h.get('content-type')"#,
        )),
        "text/html",
    );
    assert_eq!(
        text(rt.eval(r#"new window.DOMParser().parseFromString('<p>hi</p>', 'text/html').querySelector('p').textContent"#,
        )),
        "hi",
    );
    assert_eq!(
        text(rt.eval(r#"const f = new FormData(); f.append('key', 'val'); f.get('key')"#,)),
        "val",
    );
}

#[test]
fn mutation_observer_and_custom_event() {
    let rt = runtime();
    assert_eq!(
        text(rt.eval("typeof new MutationObserver(() => {})")),
        "object",
    );
    assert_eq!(
        json(rt.eval("new CustomEvent('myevent', {detail: 42}).detail")).as_deref(),
        Some("42"),
    );
}

#[test]
fn performance_real_impl() {
    let rt = runtime();
    assert!(boolean(rt.eval(
        "performance.now() >= 0 && typeof performance.now() === 'number'",
    )));
    assert!(boolean(rt.eval("performance.timeOrigin > 0")));
    let entries = text(rt.eval(
        r#"
        performance.mark('a');
        performance.measure('m', 'a');
        JSON.stringify([
          performance.getEntriesByType('mark').length,
          performance.getEntriesByName('m', 'measure').length,
        ]);
    "#,
    ));
    assert_eq!(entries, "[1,1]");
}

#[test]
fn performance_observer_fires() {
    let rt = runtime();
    let seen = text(rt.eval_async(
        r#"
        (async () => {
          const seen = [];
          const po = new PerformanceObserver(list => {
            for (const e of list.getEntries()) seen.push(e.entryType);
          });
          po.observe({entryTypes: ['mark']});
          performance.mark('watched');
          await new Promise(r => setTimeout(r, 10));
          return seen.join(',');
        })();
    "#,
    ));
    assert_eq!(seen, "mark");
}

#[test]
fn set_timeout_fires_passes_args_and_keeps_order() {
    let rt = runtime();
    assert_eq!(
        text(rt.eval_async("new Promise(resolve => setTimeout(() => resolve('ok'), 10))",)),
        "ok",
    );
    assert_eq!(
        json(rt.eval_async("new Promise(resolve => setTimeout(() => resolve(42), 0))",)).as_deref(),
        Some("42"),
    );
    assert_eq!(
        json(
            rt.eval_async("new Promise(resolve => setTimeout((a, b) => resolve(a + b), 0, 3, 4))",)
        )
        .as_deref(),
        Some("7"),
    );
    assert_eq!(
        json(rt.eval_async(
            r#"
            new Promise(resolve => {
              const log = [];
              const done = () => { if (log.length === 3) resolve(log); };
              setTimeout(() => { log.push(1); done(); }, 10);
              setTimeout(() => { log.push(2); done(); }, 20);
              setTimeout(() => { log.push(3); done(); }, 30);
            })
        "#,
        ))
        .as_deref(),
        Some("[1,2,3]"),
    );
}

#[test]
fn clear_timeout_cancels() {
    let rt = runtime();
    assert!(!boolean(rt.eval_async(
        r#"
            new Promise(resolve => {
              let fired = false;
              const id = setTimeout(() => { fired = true; }, 50);
              clearTimeout(id);
              setTimeout(() => resolve(fired), 100);
            })
        "#,
    )));
}

#[test]
fn set_interval_fires_repeatedly_then_clears() {
    let rt = runtime();
    assert_eq!(
        json(rt.eval_async(
            r#"
            new Promise(resolve => {
              let count = 0;
              const id = setInterval(() => {
                count++;
                if (count === 3) { clearInterval(id); resolve(count); }
              }, 10);
            })
        "#,
        ))
        .as_deref(),
        Some("3"),
    );
    assert!(boolean(rt.eval_async(
        r#"
            new Promise(resolve => {
              let count = 0;
              const id = setInterval(() => { count++; }, 10);
              setTimeout(() => {
                clearInterval(id);
                const current = count;
                setTimeout(() => resolve(current === count), 50);
              }, 35);
            })
        "#,
    )));
}
