//! happy-dom fixes and tweaks in patch-happy-dom.js: form-state reflection, the `:checked` /
//! `:disabled` pseudo-classes, history/location, `attachInternals`, the hx-on index, and
//! colon-containing attribute names.

mod common;

use common::{EvalExt, boolean, json, runtime, text};

#[test]
fn innerhtml_radio_mutual_exclusion() {
    let rt = runtime();
    rt.run(
        r#"document.body.innerHTML =
          '<input type="radio" name="g" checked><input type="radio" name="g" checked>'"#,
    );
    assert_eq!(
        json(rt.eval("[...document.querySelectorAll('input[type=radio]')].map(r => r.checked)",))
            .as_deref(),
        Some("[false,true]"),
    );
}

#[test]
fn innerhtml_selected_reflected() {
    let rt = runtime();
    rt.run("document.body.innerHTML = '<select><option>a</option><option selected>b</option></select>'",
    );
    assert!(boolean(
        rt.eval("document.querySelectorAll('option')[1].selected",)
    ));
}

#[test]
fn option_checked_pseudo_class_matches_selected_option() {
    let rt = runtime();
    rt.run(
        r#"document.body.innerHTML =
          '<select><option value="a">a</option><option value="b" selected>b</option></select>'"#,
    );
    assert_eq!(
        json(rt.eval("document.querySelectorAll('option:checked').length")).as_deref(),
        Some("1"),
    );
    assert_eq!(
        text(rt.eval("document.querySelector('option:checked').value")),
        "b",
    );
}

#[test]
fn option_checked_pseudo_class_updates_after_value_assignment() {
    let rt = runtime();
    rt.run(
        r#"document.body.innerHTML =
          '<select id="s"><option value="a">a</option><option value="b">b</option></select>';
          document.getElementById('s').value = 'b';"#,
    );
    assert_eq!(
        text(rt.eval("document.querySelector('option:checked').value")),
        "b",
    );
}

#[test]
fn input_checked_pseudo_class_still_works() {
    let rt = runtime();
    rt.run(r#"document.body.innerHTML = '<input type="checkbox" checked><input type="checkbox">'"#);
    assert_eq!(
        json(rt.eval("document.querySelectorAll('input:checked').length")).as_deref(),
        Some("1"),
    );
}

#[test]
fn select_value_setter_invalidates_stale_checked_query_cache() {
    let rt = runtime();
    rt.run(
        r#"document.body.innerHTML =
          '<select id="s"><option value="a">a</option><option value="b">b</option></select>'"#,
    );
    // Warm the querySelectorAll cache for this exact selector while nothing is selected.
    assert_eq!(
        json(rt.eval(r#"document.querySelectorAll("option[value='b']:checked").length"#,))
            .as_deref(),
        Some("0"),
    );
    rt.run("document.getElementById('s').value = 'b'");
    assert_eq!(
        json(rt.eval(r#"document.querySelectorAll("option[value='b']:checked").length"#,))
            .as_deref(),
        Some("1"),
    );
}

#[test]
fn history_state_updates_location() {
    for method in ["pushState", "replaceState"] {
        let rt = runtime();
        rt.run(&format!("history.{method}(null, '', '/new-path')"));
        assert_eq!(text(rt.eval("location.pathname")), "/new-path", "{method}");
    }
}

#[test]
fn history_pushstate_url_parts() {
    for (url, pathname, search) in [
        ("/page", "/page", ""),
        ("/page?q=1", "/page", "?q=1"),
        ("/a/b/c", "/a/b/c", ""),
    ] {
        let rt = runtime();
        rt.run(&format!("history.pushState(null, '', '{url}')"));
        assert_eq!(text(rt.eval("location.pathname")), pathname, "{url}");
        assert_eq!(text(rt.eval("location.search")), search, "{url}");
    }
}

#[test]
fn location_hash_change_resets_history_state() {
    let rt = runtime();
    rt.run("history.replaceState({foo: 1}, '', '/page'); location.hash = '#section';");
    assert_eq!(json(rt.eval("history.state")).as_deref(), None);
    assert_eq!(text(rt.eval("location.hash")), "#section");
    assert_eq!(text(rt.eval("location.pathname")), "/page");
}

#[test]
fn disabled_propagates_from_fieldset() {
    for tag in ["input", "button", "select", "textarea"] {
        let rt = runtime();
        rt.run(&format!(
            r#"document.body.innerHTML = "<fieldset disabled><{tag} id='x'></{tag}></fieldset>""#
        ));
        assert!(
            boolean(rt.eval("document.querySelector('#x').matches(':disabled')")),
            "{tag}",
        );
    }
}

#[test]
fn non_disabled_fieldset_does_not_disable_children() {
    let rt = runtime();
    rt.run(r#"document.body.innerHTML = "<fieldset><input id='x'></fieldset>""#);
    assert!(!boolean(
        rt.eval("document.querySelector('#x').matches(':disabled')",)
    ));
}

#[test]
fn attach_internals_set_form_value() {
    let rt = runtime();
    for (value_js, expected) in [
        ("'hello'", Some(r#""hello""#)),
        ("'42'", Some(r#""42""#)),
        ("null", None),
    ] {
        let src = format!(
            r#"
            const el = document.createElement('div');
            const internals = el.attachInternals();
            internals.setFormValue({value_js});
            el.__internalsFormValue
        "#
        );
        assert_eq!(json(rt.eval(&src)).as_deref(), expected, "{value_js}",);
    }
}

#[test]
fn getelementbyid_returns_first_in_tree_order() {
    let rt = runtime();
    rt.run(
        r#"
        document.body.innerHTML = '<div id="x">first</div>';
        const extra = document.createElement('div');
        extra.id = 'x';
        extra.textContent = 'second';
        document.documentElement.appendChild(extra);
        void 0;
    "#,
    );
    assert_eq!(
        text(rt.eval("document.getElementById('x').textContent")),
        "first",
    );
}

#[test]
fn moved_form_or_select_child_parent_identity_preserved() {
    let rt = runtime();
    for (tag, child_tag) in [("form", "input"), ("select", "option")] {
        let src = format!(
            r#"
            const helper = document.createElement('div');
            helper.innerHTML = '<{tag} id="src"><{child_tag} id="x"></{child_tag}></{tag}>';
            const moved = helper.firstChild;
            const child = moved.firstChild;
            const dest = document.createElement('div');
            dest.appendChild(moved);
            child.parentElement === dest.firstChild
        "#
        );
        assert!(boolean(rt.eval(&src)), "{tag}");
    }
}

#[test]
fn dispatch_event_sets_global_event() {
    let rt = runtime();
    for event_type in ["click", "input", "change", "custom-event"] {
        let src = format!(
            r#"
            const el = document.createElement('div');
            let captured = null;
            el.addEventListener('{event_type}', () => {{ captured = globalThis.event; }});
            const evt = new Event('{event_type}');
            el.dispatchEvent(evt);
            captured === evt
        "#
        );
        assert!(boolean(rt.eval(&src)), "{event_type}");
    }
}

#[test]
fn dispatch_event_restores_global_event_after() {
    let rt = runtime();
    let src = r#"
        const el = document.createElement('div');
        el.addEventListener('click', () => {});
        globalThis.event = 'sentinel';
        el.dispatchEvent(new Event('click'));
        globalThis.event
    "#;
    assert_eq!(text(rt.eval(src)), "sentinel");
}

const HXON_XPATH: &str =
    r#"'.//*[@*[starts-with(name(), "hx-on") or starts-with(name(), "data-hx-on")]]'"#;

#[test]
fn hxon_index_short_circuit_matches_xpath() {
    let rt = runtime();
    let src = format!(
        r#"
        document.body.innerHTML =
          '<div id="root"><b id="hit" hx-on:click="x"></b><i id="plain" class="c"></i></div>' +
          '<b id="outside" hx-on:click="y"></b>';
        const gone = document.createElement('b');
        gone.setAttribute('hx-on:click', 'z');
        const expr = new XPathEvaluator().createExpression({HXON_XPATH});
        const iter = expr.evaluate(document.getElementById('root'));
        const ids = [];
        for (let n = iter.iterateNext(); n; n = iter.iterateNext()) ids.push(n.id);
        ids
    "#
    );
    assert_eq!(json(rt.eval(&src)).as_deref(), Some(r#"["hit"]"#));
}

#[test]
fn hxon_index_picks_up_dynamic_attr_for_process_force() {
    let rt = runtime();
    let src = format!(
        r#"
        document.body.innerHTML = '<div id="d"></div>';
        const d = document.getElementById('d');
        d.setAttribute('hx-on:click', 'handler');
        const expr = new XPathEvaluator().createExpression({HXON_XPATH});
        const iter = expr.evaluate(document.body);
        iter.iterateNext()?.id
    "#
    );
    assert_eq!(text(rt.eval(&src)), "d");
}

#[test]
fn colon_attribute_setattribute_overwrites() {
    let rt = runtime();
    for attr in ["foo", ":foo", "foo:bar"] {
        let src = format!(
            r#"
            document.body.innerHTML = '<div id="a" foo="x" :foo="y"></div>';
            const d = document.getElementById('a');
            d.setAttribute('{attr}', 'first');
            d.setAttribute('{attr}', 'updated');
            d.getAttribute('{attr}')
        "#
        );
        assert_eq!(text(rt.eval(&src)), "updated", "{attr}");
    }
}
