mod common;

use _miniclient::runtime::Runtime;
use common::EvalExt;

const VENDOR_HTMX_SRC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/htmx/src");

#[test]
fn response_accepts_the_global_stream() {
    // undici's body extraction does `body instanceof ReadableStream` against the class the
    // `stream/web` bundle alias re-exports; a Response built from the global ReadableStream must
    // round-trip its bytes (guards the shim-identity trap).
    let rt = common::runtime();
    let bytes = rt.eval_promise::<Vec<u8>>(
        r#"
        (async () => {
          const stream = new ReadableStream({
            start(c) {
              c.enqueue(new Uint8Array([1, 2, 3]));
              c.close();
            },
          });
          const bytes = await new Response(stream).bytes();
          return Array.from(bytes);
        })();
    "#,
    );
    assert_eq!(bytes, vec![1, 2, 3]);
}

#[test]
fn hx_multipart_parses_chunked_straddled_boundaries() {
    // hx-multipart.js's Response.parts() over a body streamed in tiny chunks, so boundary
    // markers straddle chunk edges and chunks arrive after the parser's first read(). Every part
    // must come through whole and in order.
    let mounts = format!(r#"{{"http://localhost/vendor/": {VENDOR_HTMX_SRC:?}}}"#);
    let rt = Runtime::new("http://localhost/", &mounts);
    // document.write's scripts are parser-inserted, so they fetch automatically and execute in
    // source order -- htmx.js must run before hx-multipart.js, which calls
    // htmx.registerExtension(...) at top level and throws if `htmx` isn't defined yet. Both
    // tags in one document.write call: a second call made while the parser is still paused on
    // the first script's fetch never resumes it (a jsdom bug, not just an ordering nicety).
    rt.run_promise(
        r#"
        const loaded = new Promise((resolve) => window.addEventListener('load', resolve, { once: true }));
        document.open();
        document.write(`
            <script src="http://localhost/vendor/htmx.js"></script>
            <script src="http://localhost/vendor/ext/hx-multipart.js"></script>
        `);
        document.close();
        loaded.then(() => true);
    "#,
    );
    let parts = rt.eval_promise::<Vec<String>>(
        r#"
        (async () => {
          const CRLF = String.fromCharCode(13, 10);
          const boundary = 'BoundaryX';
          const parts = ['first part body', 'second part is a bit longer than the first'];
          let payload = '';
          for (const p of parts)
            payload += '--' + boundary + CRLF + 'Content-Type: text/plain' + CRLF + CRLF + p + CRLF;
          payload += '--' + boundary + '--' + CRLF;
          const bytes = new TextEncoder().encode(payload);
          const stream = new ReadableStream({
            start(c) {
              let i = 0;
              const push = () => {
                if (i >= bytes.length) {
                  c.close();
                  return;
                }
                c.enqueue(bytes.slice(i, i + 5)); // 5-byte chunks straddle boundaries
                i += 5;
                setTimeout(push, 1);
              };
              setTimeout(push, 1);
            },
          });
          const res = new Response(stream, {
            headers: {'content-type': `multipart/mixed; boundary=${boundary}`},
          });
          const out = [];
          for await (const part of res.parts()) out.push(await part.text());
          return out;
        })();
    "#,
    );
    assert_eq!(
        parts,
        vec![
            "first part body",
            "second part is a bit longer than the first"
        ],
    );
}
