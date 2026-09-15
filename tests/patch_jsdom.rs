//! jsdom-only fixes applied in bootstrap.js's registerWindowGlobals (see the jsdom-port
//! wayfinder, `.scratch/jsdom-port/`).

mod common;

use common::{EvalExt, runtime};

#[test]
fn xpath_expression_evaluate_defaults_type_to_any_type() {
    let rt = runtime();
    let js = r#"
        document.body.innerHTML =
          '<div id="root"><b id="hit" hx-on:click="x"></b><i id="plain"></i></div>';
        const expr = document.createExpression(
          './/*[@*[starts-with(name(), "hx-on")]]',
          null,
        );
        const iter = expr.evaluate(document.getElementById('root'));
        const ids = [];
        for (let n = iter.iterateNext(); n; n = iter.iterateNext()) ids.push(n.id);
        ids;
    "#;
    assert_eq!(rt.eval_json(js).as_deref(), Some(r#"["hit"]"#));
}
