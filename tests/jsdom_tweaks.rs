//! jsdom behaviour mini leans on: form-state reflection, the `:checked` / `:disabled`
//! pseudo-classes, history/location, `attachInternals`, and colon-containing attribute names.

mod common;

use common::fetch_mock::{MockFetchBackend, runtime_with_mock};
use common::{EvalExt, Runtime, runtime};

#[test]
fn innerhtml_radio_mutual_exclusion() {
    let rt = runtime();
    rt.run(
        r#"document.body.innerHTML =
  '<input type="radio" name="g" checked><input type="radio" name="g" checked>'"#,
    );
    assert_eq!(
        rt.eval_json("[...document.querySelectorAll('input[type=radio]')].map(r => r.checked)",)
            .as_deref(),
        Some("[false,true]"),
    );
}

#[test]
fn innerhtml_selected_reflected() {
    let rt = runtime();
    rt.run(
        "document.body.innerHTML =
  '<select><option>a</option><option selected>b</option></select>'",
    );
    assert!(rt.eval::<bool>("document.querySelectorAll('option')[1].selected",));
}

#[test]
fn option_checked_pseudo_class_matches_selected_option() {
    let rt = runtime();
    rt.run(
        r#"document.body.innerHTML =
  '<select><option value="a">a</option><option value="b" selected>b</option></select>'"#,
    );
    assert_eq!(
        rt.eval_json("document.querySelectorAll('option:checked').length")
            .as_deref(),
        Some("1"),
    );
    assert_eq!(
        rt.eval::<String>("document.querySelector('option:checked').value"),
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
        rt.eval::<String>("document.querySelector('option:checked').value"),
        "b",
    );
}

#[test]
fn input_checked_pseudo_class_still_works() {
    let rt = runtime();
    rt.run(r#"document.body.innerHTML = '<input type="checkbox" checked><input type="checkbox">'"#);
    assert_eq!(
        rt.eval_json("document.querySelectorAll('input:checked').length")
            .as_deref(),
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
        rt.eval_json(r#"document.querySelectorAll("option[value='b']:checked").length"#,)
            .as_deref(),
        Some("0"),
    );
    rt.run("document.getElementById('s').value = 'b'");
    assert_eq!(
        rt.eval_json(r#"document.querySelectorAll("option[value='b']:checked").length"#,)
            .as_deref(),
        Some("1"),
    );
}

#[test]
fn history_state_updates_location() {
    for method in ["pushState", "replaceState"] {
        let rt = runtime();
        rt.run(&format!("history.{method}(null, '', '/new-path')"));
        assert_eq!(
            rt.eval::<String>("location.pathname"),
            "/new-path",
            "{method}"
        );
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
        assert_eq!(rt.eval::<String>("location.pathname"), pathname, "{url}");
        assert_eq!(rt.eval::<String>("location.search"), search, "{url}");
    }
}

#[test]
fn history_length_starts_at_one() {
    let rt = runtime();
    assert_eq!(rt.eval::<i32>("history.length"), 1);
}

#[test]
fn history_length_counts_pushstate_entries() {
    let rt = runtime();
    rt.run("history.pushState(null, '', '/a')");
    assert_eq!(rt.eval::<i32>("history.length"), 2);
    rt.run("history.pushState(null, '', '/b')");
    assert_eq!(rt.eval::<i32>("history.length"), 3);
}

/// A runtime whose fetch serves a stub page at each of `paths` under http://localhost.
fn runtime_serving(paths: &[&str]) -> Runtime {
    let mock = MockFetchBackend::new();
    for path in paths {
        mock.text(
            &format!("http://localhost{path}"),
            &format!("<html><body><p>{path}</p></body></html>"),
        );
    }
    runtime_with_mock(&mock)
}

/// Same mechanism `Page.goto()` (page.py) and a real link click use -- drives
/// `op_request_navigation` directly, same as `tests/navigation_fetch.rs`, so this file's
/// history tests exercise the real isolate-navigation path, not a stand-in for it.
fn goto(rt: &Runtime, url: &str) {
    rt.run(&format!(
        "Deno.core.ops.op_request_navigation('{url}', 'push'); void 0"
    ));
}

fn history_go(rt: &Runtime, delta: i32) {
    rt.run(&format!("history.go({delta})"));
}

fn href(rt: &Runtime) -> String {
    rt.eval("location.href")
}

#[test]
fn history_length_counts_same_document_entries_across_navigations() {
    let rt = runtime_serving(&["/a", "/b"]);
    goto(&rt, "http://localhost/a");
    let before = rt.eval::<i32>("history.length");
    rt.run("history.pushState(null, '', '/a2'); history.pushState(null, '', '/a3')");
    goto(&rt, "http://localhost/b");
    assert_eq!(rt.eval::<i32>("history.length"), before + 3);
}

#[test]
fn history_back_restores_pushed_entry_and_state() {
    let rt = runtime_serving(&["/a", "/a2", "/b"]);
    goto(&rt, "http://localhost/a");
    rt.run("history.pushState({n: 1}, '', '/a2')");
    goto(&rt, "http://localhost/b");

    history_go(&rt, -1);
    assert_eq!(href(&rt), "http://localhost/a2");
    assert_eq!(rt.eval::<i32>("history.state.n"), 1);

    history_go(&rt, -1);
    assert_eq!(href(&rt), "http://localhost/a");

    history_go(&rt, 2);
    assert_eq!(href(&rt), "http://localhost/b");
}

#[test]
fn history_go_counts_entries_pushed_in_current_document() {
    let rt = runtime_serving(&["/a", "/b"]);
    goto(&rt, "http://localhost/a");
    goto(&rt, "http://localhost/b");
    rt.run("history.pushState(null, '', '/b2')");

    history_go(&rt, -2);
    assert_eq!(href(&rt), "http://localhost/a");
}

#[test]
fn history_back_restores_hash_entry() {
    let rt = runtime_serving(&["/a", "/b"]);
    goto(&rt, "http://localhost/a");
    rt.run("location.hash = '#x'");
    goto(&rt, "http://localhost/b");

    history_go(&rt, -1);
    assert_eq!(href(&rt), "http://localhost/a#x");
}

#[test]
fn history_replace_state_does_not_add_entry() {
    let rt = runtime_serving(&["/a", "/b"]);
    goto(&rt, "http://localhost/a");
    let before = rt.eval::<i32>("history.length");
    rt.run("history.replaceState({r: 1}, '', '/a-replaced')");
    goto(&rt, "http://localhost/b");
    assert_eq!(rt.eval::<i32>("history.length"), before + 1);
}

#[test]
fn location_replace_does_not_grow_history() {
    let rt = runtime_serving(&["/a", "/next"]);
    goto(&rt, "http://localhost/a");
    rt.run("history.pushState({n: 1}, '', '/prev')");
    let before = rt.eval::<i32>("history.length");

    rt.run("location.replace('/next')");

    assert_eq!(href(&rt), "http://localhost/next");
    assert_eq!(rt.eval::<i32>("history.length"), before);
}

#[test]
fn document_write_does_not_add_history_entry() {
    let rt = runtime();
    let before = rt.eval::<i32>("history.length");
    rt.run_promise("__mini.documentWrite('<p>x</p>')");
    assert_eq!(rt.eval::<i32>("history.length"), before);
}

#[test]
fn location_hash_change_resets_history_state() {
    let rt = runtime();
    rt.run("history.replaceState({foo: 1}, '', '/page'); location.hash = '#section'");
    assert_eq!(rt.eval_json("history.state").as_deref(), None);
    assert_eq!(rt.eval::<String>("location.hash"), "#section");
    assert_eq!(rt.eval::<String>("location.pathname"), "/page");
}

#[test]
fn disabled_propagates_from_fieldset() {
    for tag in ["input", "button", "select", "textarea"] {
        let rt = runtime();
        rt.run(&format!(
            r#"document.body.innerHTML =
  "<fieldset disabled><{tag} id='x'></{{tag}}></fieldset>""#
        ));
        assert!(
            rt.eval::<bool>("document.querySelector('#x').matches(':disabled')"),
            "{tag}",
        );
    }
}

#[test]
fn non_disabled_fieldset_does_not_disable_children() {
    let rt = runtime();
    rt.run(r#"document.body.innerHTML = "<fieldset><input id='x'></fieldset>""#);
    assert!(!rt.eval::<bool>("document.querySelector('#x').matches(':disabled')",));
}

#[test]
fn attach_internals_set_form_value() {
    let rt = runtime();
    for (value_js, expected) in [
        ("'hello'", Some(r#""hello""#)),
        ("'42'", Some(r#""42""#)),
        ("null", None),
    ] {
        let js = format!(
            r#"
            const tag = 'mini-test-fa-{value_js_id}';
            class FA extends HTMLElement {{
                static formAssociated = true;
                #internals = this.attachInternals();
                setValue(value) {{ this.#internals.setFormValue(value); }}
            }}
            customElements.define(tag, FA);
            const form = document.createElement('form');
            const el = document.createElement(tag);
            el.setAttribute('name', 'field');
            el.setValue({value_js});
            form.appendChild(el);
            new FormData(form).get('field');
        "#,
            value_js_id = value_js
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>(),
        );
        assert_eq!(rt.eval_json(&js).as_deref(), expected, "{value_js}",);
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
        rt.eval::<String>("document.getElementById('x').textContent"),
        "first",
    );
}

#[test]
fn moved_form_or_select_child_parent_identity_preserved() {
    let rt = runtime();
    for (tag, child_tag) in [("form", "input"), ("select", "option")] {
        let js = format!(
            r#"
            const helper = document.createElement('div');
            helper.innerHTML =
              '<{tag} id="src"><{child_tag} id="x"></{child_tag}></{tag}>';
            const moved = helper.firstChild;
            const child = moved.firstChild;
            const dest = document.createElement('div');
            dest.appendChild(moved);
            child.parentElement === dest.firstChild;
        "#
        );
        assert!(rt.eval::<bool>(&js), "{tag}");
    }
}

#[test]
fn dispatch_event_sets_global_event() {
    let rt = runtime();
    for event_type in ["click", "input", "change", "custom-event"] {
        let js = format!(
            r#"
            const el = document.createElement('div');
            let captured = null;
            el.addEventListener('{event_type}', () => {{
              captured = globalThis.event;
            }});
            const evt = new Event('{event_type}');
            el.dispatchEvent(evt);
            captured === evt;
        "#
        );
        assert!(rt.eval::<bool>(&js), "{event_type}");
    }
}

#[test]
fn global_event_is_idle_undefined_and_restores_across_nested_dispatch() {
    let rt = runtime();
    let js = r#"
        const el = document.createElement('div');
        let outerType = null;
        el.addEventListener('click', () => {
            const inner = document.createElement('div');
            inner.addEventListener('x', () => {});
            inner.dispatchEvent(new Event('x'));
            outerType = globalThis.event.type;   // outer event restored after the nested one
        });
        const idleBefore = globalThis.event === undefined;
        el.dispatchEvent(new Event('click'));
        JSON.stringify([idleBefore, outerType, globalThis.event === undefined]);
    "#;
    assert_eq!(rt.eval::<String>(js), r#"[true,"click",true]"#);
}

// hx-on index short-circuit behaviour is covered by patch_jsdom.rs (patch-jsdom-hxon-index.js
// is the current implementation; this file only holds jsdom-native-engine tweaks now).

#[test]
fn colon_attribute_setattribute_overwrites() {
    let rt = runtime();
    for attr in ["foo", ":foo", "foo:bar"] {
        let js = format!(
            r#"
            document.body.innerHTML = '<div id="a" foo="x" :foo="y"></div>';
            const d = document.getElementById('a');
            d.setAttribute('{attr}', 'first');
            d.setAttribute('{attr}', 'updated');
            d.getAttribute('{attr}');
        "#
        );
        assert_eq!(rt.eval::<String>(&js), "updated", "{attr}");
    }
}
