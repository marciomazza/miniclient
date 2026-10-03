pub static DEFAULT_SNAPSHOT: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/DEFAULT_SNAPSHOT.bin"));

#[cfg(test)]
mod tests {
    use deno_core::{JsRuntime, RuntimeOptions};

    use crate::runtime::{extensions, init_platform};

    /// Boots a snapshot and reads one expression out of it -- the only proof that a blob is
    /// restorable, not merely non-empty.
    fn eval_in_snapshot(blob: &'static [u8], expr: &'static str) -> String {
        init_platform();
        let mut js = JsRuntime::new(RuntimeOptions {
            startup_snapshot: Some(blob),
            extensions: extensions(),
            ..Default::default()
        });
        let value = js.execute_script("<test>", expr).unwrap();
        deno_core::scope!(scope, js);
        deno_core::v8::Local::new(scope, value).to_rust_string_lossy(scope)
    }

    #[test]
    fn default_snapshot_boots_and_exposes_dom_globals() {
        // FormData isn't checked here: it only becomes available once bootstrap.js's
        // registerWindowGlobals runs against a real Window, which happens per-`Runtime`, not
        // as part of the snapshot itself (see `runtime_apis.rs`'s `headers_dom_parser_and_form_data`).
        assert_eq!(
            eval_in_snapshot(
                crate::snapshot::DEFAULT_SNAPSHOT,
                "[typeof Request, typeof Response, typeof JSDOM].join()"
            ),
            "function,function,function"
        );
    }

    #[test]
    fn default_snapshot_restores_with_the_dom_ops() {
        assert_eq!(
            eval_in_snapshot(
                crate::snapshot::DEFAULT_SNAPSHOT,
                "String(Deno.core.ops.op_dom_create(1, 'div', ''))"
            ),
            "0"
        );
    }
}
