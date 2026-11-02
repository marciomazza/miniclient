//! Fragment parsing (`innerHTML`, `outerHTML`, `insertAdjacentHTML`,
//! `createContextualFragment`) builds the tree in the arena and creates every impl from it. The
//! tree is read back through the public API only, so a node without an impl, or with state its
//! insertion steps never derived, shows up here.
#![allow(clippy::unwrap_used)]

mod common;

use common::{EvalExt, runtime};

/// `dump(node)`: the subtree as nested arrays of public node properties.
const DUMP: &str = r#"
const dump = (n) => {
    if (n.nodeType === 1) {
        const attrs = [...n.attributes].map((a) => [a.namespaceURI, a.prefix, a.localName, a.value]);
        return [n.namespaceURI, n.localName, attrs, [...n.childNodes].map(dump)];
    }
    if (n.nodeType === 11) return [...n.childNodes].map(dump);
    return [n.nodeType, n.data];
};
"#;

#[track_caller]
fn assert_dump(setup: &str, expected: &str) {
    let rt = runtime();
    let got: serde_json::Value = rt.eval(&format!("{DUMP}\nlet node;\n{setup}\ndump(node)"));
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(got, expected);
}

#[test]
fn inner_html_builds_text_comments_and_attributes() {
    assert_dump(
        r#"node = document.createElement("div");
        node.innerHTML = '<p class="a" id=x>one &amp; <b>two</b></p><!-- c -->tail';"#,
        r#"[ "http://www.w3.org/1999/xhtml", "div", [], [
            ["http://www.w3.org/1999/xhtml", "p", [[null, null, "class", "a"], [null, null, "id", "x"]], [
                [3, "one & "],
                ["http://www.w3.org/1999/xhtml", "b", [], [[3, "two"]]]
            ]],
            [8, " c "],
            [3, "tail"]
        ]]"#,
    );
}

#[test]
fn context_element_picks_the_insertion_mode_and_namespace() {
    assert_dump(
        r#"node = document.createElement("tbody");
        node.innerHTML = "<tr><td>a</td></tr>";"#,
        r#"["http://www.w3.org/1999/xhtml", "tbody", [], [
            ["http://www.w3.org/1999/xhtml", "tr", [], [
                ["http://www.w3.org/1999/xhtml", "td", [], [[3, "a"]]]
            ]]
        ]]"#,
    );
    assert_dump(
        r##"node = document.createElementNS("http://www.w3.org/2000/svg", "svg");
        node.innerHTML = '<a xlink:href="#h"><foreignObject><p>x</p></foreignObject></a>';"##,
        r##"["http://www.w3.org/2000/svg", "svg", [], [
            ["http://www.w3.org/2000/svg", "a", [["http://www.w3.org/1999/xlink", "xlink", "href", "#h"]], [
                ["http://www.w3.org/2000/svg", "foreignObject", [], [
                    ["http://www.w3.org/1999/xhtml", "p", [], [[3, "x"]]]
                ]]
            ]]
        ]]"##,
    );
}

#[test]
fn a_form_context_drops_a_nested_form_tag() {
    assert_dump(
        r#"const form = document.createElement("form");
        node = form.appendChild(document.createElement("div"));
        node.innerHTML = "<form><input></form>";"#,
        r#"["http://www.w3.org/1999/xhtml", "div", [], [
            ["http://www.w3.org/1999/xhtml", "input", [], []]
        ]]"#,
    );
}

#[test]
fn outer_html_insert_adjacent_html_and_contextual_fragments() {
    assert_dump(
        r#"node = document.createElement("div");
        node.innerHTML = "<i>old</i>";
        node.firstChild.outerHTML = "<b>new</b><u></u>";
        node.insertAdjacentHTML("afterbegin", "<s>first</s>");
        node.lastChild.insertAdjacentHTML("afterend", "last");
        const range = document.createRange();
        range.selectNodeContents(node);
        node.append(range.createContextualFragment("<em>ctx</em>"));"#,
        r#"["http://www.w3.org/1999/xhtml", "div", [], [
            ["http://www.w3.org/1999/xhtml", "s", [], [[3, "first"]]],
            ["http://www.w3.org/1999/xhtml", "b", [], [[3, "new"]]],
            ["http://www.w3.org/1999/xhtml", "u", [], []],
            [3, "last"],
            ["http://www.w3.org/1999/xhtml", "em", [], [[3, "ctx"]]]
        ]]"#,
    );
}

#[test]
fn template_content_goes_into_the_content_fragment() {
    let rt = runtime();
    let got: Vec<serde_json::Value> = rt.eval(
        r#"document.body.innerHTML =
            '<template id="t"><b class="in">x</b><template><i></i></template></template>';
        const t = document.getElementById("t");
        const inner = t.content.lastChild;
        const before = [t.childNodes.length, t.content.firstChild.className,
            inner.content.firstChild.localName, document.querySelector(".in"),
            t.content.querySelector(".in") === t.content.firstChild,
            t.content.ownerDocument !== document, t.content.firstChild.ownerDocument === t.content.ownerDocument];
        t.innerHTML = "<u></u>";
        [...before, t.content.firstChild.localName, t.childNodes.length]"#,
    );
    assert_eq!(
        serde_json::Value::Array(got),
        serde_json::json!([0, "in", "i", null, true, true, true, "u", 0])
    );
}

#[test]
fn inner_html_scripts_do_not_run_and_dom_method_scripts_do() {
    let rt = runtime();
    let got: Vec<bool> = rt.eval(
        r#"globalThis.ran = [];
        document.body.innerHTML = "<script>ran.push('innerHTML')</script>";
        document.body.insertAdjacentHTML("beforeend", "<script>ran.push('adjacent')</script>");
        const script = document.createElement("script");
        script.textContent = "ran.push('dom')";
        document.body.append(script);
        ["innerHTML", "adjacent", "dom"].map((k) => ran.includes(k))"#,
    );
    assert_eq!(got, [false, false, true]);
}

#[test]
fn insertion_and_attribute_steps_derive_element_state() {
    let rt = runtime();
    let got: serde_json::Value = rt.eval(
        r#"document.body.innerHTML = `<form id="f">
            <textarea>abc</textarea>
            <select><option>a</option><option selected>b</option></select>
            <input type="checkbox" checked value="v">
        </form><noscript><b>raw</b></noscript>`;
        const f = document.getElementById("f");
        const [textarea, select, input] = f.elements;
        [textarea.value, select.value, input.checked, input.value, input.form === f,
            f.elements.length, document.querySelector("noscript").firstChild.nodeType]"#,
    );
    assert_eq!(got, serde_json::json!(["abc", "b", true, "v", true, 3, 3]));
}

#[test]
fn custom_elements_in_parsed_html_upgrade_and_connect() {
    let rt = runtime();
    let got: Vec<String> = rt.eval(
        r#"const log = [];
        customElements.define("x-parsed", class extends HTMLElement {
            constructor() { super(); log.push("constructed"); }
            connectedCallback() { log.push("connected:" + this.getAttribute("a")); }
        });
        document.body.innerHTML = '<x-parsed a="1"></x-parsed>';
        log"#,
    );
    assert_eq!(got, ["constructed", "connected:1"]);
}

#[test]
fn custom_elements_upgrade_in_tree_order() {
    let rt = runtime();
    let got: Vec<String> = rt.eval_promise(
        r#"const log = [];
        for (const name of ["x-outer", "x-inner"]) {
            customElements.define(name, class extends HTMLElement {
                constructor() { super(); log.push(name + ":" + this.getAttribute("n")); }
            });
        }
        const markup = '<x-outer n="1"><x-inner n="2"></x-inner></x-outer><x-inner n="3"></x-inner>';
        document.body.innerHTML = markup;
        const range = document.createRange();
        range.selectNodeContents(document.body);
        document.body.append(range.createContextualFragment(markup.replaceAll('n="', 'n="c')));
        Promise.resolve().then(() => log)"#,
    );
    assert_eq!(
        got,
        [
            "x-outer:1",
            "x-inner:2",
            "x-inner:3",
            "x-outer:c1",
            "x-inner:c2",
            "x-inner:c3"
        ]
    );
}

#[test]
fn parsed_nodes_keep_their_identity() {
    let rt = runtime();
    let same: bool = rt.eval(
        r#"const div = document.createElement("div");
        div.innerHTML = "<p><b></b></p>";
        const b = div.querySelector("b");
        const map = new WeakMap([[b, 1]]);
        div.firstChild.firstChild === b && b.parentNode === div.firstChild && map.get(div.firstChild.firstChild) === 1"#,
    );
    assert!(same);
}

#[test]
fn materialized_node_count_covers_the_parsed_fragment() {
    let rt = runtime();
    let added: u32 = rt.eval(
        r#"const div = document.createElement("div");
        document.body.append(div);
        const before = __materializedNodeCount();
        div.innerHTML = "<p>a<b>b</b></p><!--c-->";
        __materializedNodeCount() - before"#,
    );
    // the fragment, <p>, "a", <b>, "b" and the comment
    assert_eq!(added, 6);
}

#[test]
fn parsed_svg_elements_get_the_svg_interface() {
    let rt = runtime();
    let out: Vec<String> = rt.eval(
        r##"document.body.innerHTML = '<svg id="s"><circle id="c"></circle></svg>';
        const c = document.querySelector("circle");
        [c.id, String(c instanceof SVGElement), String(c instanceof HTMLElement),
            c.namespaceURI, document.querySelectorAll("svg *").length + ""]"##,
    );
    assert_eq!(
        out,
        ["c", "true", "false", "http://www.w3.org/2000/svg", "1"]
    );
}
