//! The deno_web, happy-dom and node-polyfill globals the default snapshot boots, exercised
//! through `send_eval`.

mod common;

use common::{boolean, eval, eval_async, json, run, runtime, text};

#[test]
fn window_and_document_basics() {
    let rt = runtime();
    assert_eq!(text(eval(&rt, "typeof window")), "object");
    assert_eq!(
        text(eval(&rt, "document.createElement('div').tagName")),
        "DIV"
    );
    assert!(!boolean(eval(&rt, "new AbortController().signal.aborted")));
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
            boolean(eval(
                &rt,
                &format!("typeof globalThis.{name} !== 'undefined'")
            )),
            "{name}",
        );
    }
}

#[test]
fn buffer() {
    let rt = runtime();
    for (src, want) in [
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
        assert_eq!(text(eval(&rt, src)), want, "{src}");
    }
    assert!(boolean(eval(&rt, "Buffer.isBuffer(Buffer.alloc(4))")));
    assert!(!boolean(eval(&rt, "Buffer.isBuffer(new Uint8Array(4))")));
}

#[test]
fn dom_manipulation() {
    let rt = runtime();
    run(
        &rt,
        r#"document.body.innerHTML = '<div id="x"><span class="y">hi</span></div>'"#,
    );
    assert_eq!(
        text(eval(&rt, "document.querySelector('#x .y').textContent")),
        "hi",
    );

    run(
        &rt,
        "document.body.innerHTML = '<ul><li>a</li><li>b</li><li>c</li></ul>'",
    );
    assert_eq!(
        json(eval(&rt, "document.querySelectorAll('li').length")).as_deref(),
        Some("3"),
    );

    run(&rt, r#"document.body.innerHTML = '<p id="p1">text</p>'"#);
    assert_eq!(
        text(eval(&rt, "document.getElementById('p1').innerHTML")),
        "text",
    );

    let href = text(eval(
        &rt,
        "const a = document.createElement('a'); a.href = 'http://z.com'; a.href",
    ));
    assert!(href.contains("z.com"), "{href:?}");
}

#[test]
fn happy_dom_globals_are_functions() {
    let rt = runtime();
    for src in [
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
        assert_eq!(text(eval(&rt, src)), "function", "{src}");
    }
}

#[test]
fn headers_dom_parser_and_form_data() {
    let rt = runtime();
    assert_eq!(
        text(eval(
            &rt,
            r#"const h = new Headers({'content-type': 'text/html'}); h.get('content-type')"#,
        )),
        "text/html",
    );
    assert_eq!(
        text(eval(
            &rt,
            r#"new window.DOMParser().parseFromString('<p>hi</p>', 'text/html').querySelector('p').textContent"#,
        )),
        "hi",
    );
    assert_eq!(
        text(eval(
            &rt,
            r#"const f = new FormData(); f.append('key', 'val'); f.get('key')"#,
        )),
        "val",
    );
}

#[test]
fn mutation_observer_and_custom_event() {
    let rt = runtime();
    assert_eq!(
        text(eval(&rt, "typeof new MutationObserver(() => {})")),
        "object",
    );
    assert_eq!(
        json(eval(&rt, "new CustomEvent('myevent', {detail: 42}).detail")).as_deref(),
        Some("42"),
    );
}

#[test]
fn performance_real_impl() {
    let rt = runtime();
    assert!(boolean(eval(
        &rt,
        "performance.now() >= 0 && typeof performance.now() === 'number'",
    )));
    assert!(boolean(eval(&rt, "performance.timeOrigin > 0")));
    let entries = text(eval(
        &rt,
        r#"
        performance.mark('a');
        performance.measure('m', 'a');
        JSON.stringify([
            performance.getEntriesByType('mark').length,
            performance.getEntriesByName('m', 'measure').length,
        ])
    "#,
    ));
    assert_eq!(entries, "[1,1]");
}

#[test]
fn performance_observer_fires() {
    let rt = runtime();
    let seen = text(eval_async(
        &rt,
        r#"
        (async () => {
            const seen = [];
            const po = new PerformanceObserver((list) => {
                for (const e of list.getEntries()) seen.push(e.entryType);
            });
            po.observe({ entryTypes: ['mark'] });
            performance.mark('watched');
            await new Promise((r) => setTimeout(r, 10));
            return seen.join(',');
        })()
    "#,
    ));
    assert_eq!(seen, "mark");
}

#[test]
fn set_timeout_fires_passes_args_and_keeps_order() {
    let rt = runtime();
    assert_eq!(
        text(eval_async(
            &rt,
            "new Promise(resolve => setTimeout(() => resolve('ok'), 10))",
        )),
        "ok",
    );
    assert_eq!(
        json(eval_async(
            &rt,
            "new Promise(resolve => setTimeout(() => resolve(42), 0))",
        ))
        .as_deref(),
        Some("42"),
    );
    assert_eq!(
        json(eval_async(
            &rt,
            "new Promise(resolve => setTimeout((a, b) => resolve(a + b), 0, 3, 4))",
        ))
        .as_deref(),
        Some("7"),
    );
    assert_eq!(
        json(eval_async(
            &rt,
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
    assert!(!boolean(eval_async(
        &rt,
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
        json(eval_async(
            &rt,
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
    assert!(boolean(eval_async(
        &rt,
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
