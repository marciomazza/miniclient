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

#[test]
fn hxon_index_short_circuit_sees_attributes_set_after_parse() {
    let rt = runtime();
    let js = r#"
        document.body.innerHTML =
          '<div id="root"><b id="plain"></b><i id="other"></i></div>';
        // Not present at parse time -- only the _attrModified hook (not a one-time parse
        // walk) can have added #dynamic to the index.
        document.getElementById('plain').setAttribute('hx-on:click', 'x');
        const expr = document.createExpression(
          './/*[@*[starts-with(name(), "hx-on")]]',
          null,
        );
        const iter = expr.evaluate(document.getElementById('root'));
        const ids = [];
        for (let n = iter.iterateNext(); n; n = iter.iterateNext()) ids.push(n.id);
        ids;
    "#;
    assert_eq!(rt.eval_json(js).as_deref(), Some(r#"["plain"]"#));
}
