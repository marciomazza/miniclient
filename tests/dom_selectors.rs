//! `querySelector`, `querySelectorAll`, `closest` and `matches` through the public API. Most
//! selectors match natively over the arena; the ones the native matcher declines (`:lang`,
//! state pseudo-classes, ...) go to `dom-selector`. Either way the answers are a browser's.

mod common;

use std::collections::HashMap;

use common::{EvalExt, js_error, runtime};

const PAGE: &str = r##"
    document.body.innerHTML = `<div id="root">
      <section id="a" class="Card x">
        <p id="p1" class="lead" data-role="Panel Main" lang="en-US">one</p>
        <p id="p2" title="hello-world">two</p>
        <ul id="u"><li id="l1">x</li><li id="l2" class="sel">y</li><li id="l3"></li></ul>
        <input id="i1" type="TEXT">
      </section>
      <section id="b"><div id="e"></div><div id="e2"><!--c--></div><span id="s1"></span></section>
    </div>`;
    const ids = (list) => [...list].map((el) => el.id || el.localName).join(" ");
"##;

/// `(selector, ids of root.querySelectorAll(selector))`, in document order.
const CASES: &[(&str, &str)] = &[
    ("p", "p1 p2"),
    ("P", "p1 p2"),
    ("*|p", "p1 p2"),
    ("#a", "a"),
    (".lead", "p1"),
    (".lead.missing", ""),
    ("section.Card", "a"),
    ("[data-role]", "p1"),
    ("[DATA-ROLE]", "p1"),
    ("[data-role='Panel Main']", "p1"),
    ("[data-role='panel main' i]", "p1"),
    ("[data-role='panel main' s]", ""),
    ("[data-role~=Main]", "p1"),
    ("[lang|=en]", "p1"),
    ("[title^=hello]", "p2"),
    ("[title$=world]", "p2"),
    ("[title*='o-w']", "p2"),
    ("section p", "p1 p2"),
    ("section > p", "p1 p2"),
    ("#p1 + p", "p2"),
    ("#p1 ~ *", "p2 u i1"),
    ("#u > li, #a", "a l1 l2 l3"),
    (":is(#e, #p2)", "p2 e"),
    (":where(ul) li.sel", "l2"),
    ("li:not(.sel)", "l1 l3"),
    ("section:has(> ul)", "a"),
    ("section:has(span)", "b"),
    ("li:first-child", "l1"),
    ("li:last-child", "l3"),
    ("#b > :first-of-type", "e s1"),
    ("div:last-of-type", "e2"),
    ("span:only-of-type", "s1"),
    ("li:only-child", ""),
    ("li:nth-child(2)", "l2"),
    ("li:nth-child(odd)", "l1 l3"),
    ("li:nth-last-child(1)", "l3"),
    ("div:nth-of-type(2)", "e2"),
    ("div:nth-last-of-type(2)", "e"),
    (":nth-child(2 of .sel, #l3)", "l3"),
    ("#b :empty", "e e2 s1"),
    ("li:empty", "l3"),
    (":root", ""),
    ("|p", ""),
    // Legacy HTML attribute whose value matches case-insensitively.
    ("[type=text]", "i1"),
];

#[test]
fn selectors_match_in_document_order() {
    let rt = runtime();
    let selectors: Vec<&str> = CASES.iter().map(|(s, _)| *s).collect();
    let got: HashMap<String, String> = rt.eval(&format!(
        r##"{PAGE}
        const root = document.getElementById("root");
        Object.fromEntries({selectors:?}.map((s) => [s, ids(root.querySelectorAll(s))]))"##
    ));
    let wrong: Vec<_> = CASES
        .iter()
        .filter(|(s, want)| got[*s] != *want)
        .map(|(s, want)| format!("{s}: want {want:?}, got {:?}", got[*s]))
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn scope_closest_and_matches() {
    let rt = runtime();
    let out: Vec<serde_json::Value> = rt.eval(&format!(
        r##"{PAGE}
        const a = document.getElementById("a"), p1 = document.getElementById("p1");
        [ids(a.querySelectorAll(":scope > p")), ids(a.querySelectorAll("section p")),
            ids(a.querySelectorAll("section")), ids(document.querySelectorAll(":scope")),
            document.querySelector(":root").localName, a.querySelector("li").id,
            p1.closest("p").id, p1.closest("section").id, p1.closest("#root > section").id,
            p1.closest("ul"), p1.matches("section > p.lead"), p1.matches("section"),
            p1.matches(":scope"), p1.webkitMatchesSelector(".lead"), p1.closest(":scope").id]"##
    ));
    assert_eq!(
        serde_json::Value::Array(out).to_string(),
        r##"["p1 p2","p1 p2","","html","html","l1","p1","a","a",null,true,false,true,true,"p1"]"##
    );
}

#[test]
fn detached_trees_fragments_and_template_content() {
    let rt = runtime();
    let out: Vec<serde_json::Value> = rt.eval(
        r##"
        const d = document.createElement("div");
        d.innerHTML = "<p class=x><b></b></p>";
        const b = d.querySelector("p > b");
        const frag = document.createDocumentFragment();
        frag.append(document.createElement("i"), document.createElement("i"));
        frag.lastChild.className = "x";
        const t = document.createElement("template");
        t.innerHTML = "<li class=t>1</li><li>2</li>";
        document.body.append(t);
        const before = [document.querySelector("li.t"), t.content.querySelectorAll("li").length,
            t.content.querySelector("li:first-child.t").textContent];
        document.body.append(t.content);
        [d.matches("div"), d.matches(":root"), b.closest("div") === d, b.matches("div b"),
            frag.querySelectorAll("i").length, frag.querySelector(".x") === frag.lastChild,
            ...before, document.querySelectorAll("body > li.t").length,
            document.querySelector("li.t + li").textContent]"##,
    );
    assert_eq!(
        serde_json::Value::Array(out).to_string(),
        r##"[true,false,true,true,2,true,null,2,"1",1,"2"]"##
    );
}

#[test]
fn case_rules_follow_the_document_type() {
    let rt = runtime();
    let out: Vec<usize> = rt.eval(
        r##"
        const xml = new DOMParser().parseFromString(
            '<root><Item Kind="A"/><item/></root>', "application/xml");
        const html = document.createElement("div");
        html.innerHTML = '<svg><foreignObject></foreignObject></svg><DIV Data-X="1"></DIV>';
        [xml.querySelectorAll("Item").length, xml.querySelectorAll("item").length,
            xml.querySelectorAll("ITEM").length, xml.querySelectorAll("[Kind]").length,
            xml.querySelectorAll("[kind]").length, html.querySelectorAll("DIV").length,
            html.querySelectorAll("[data-x]").length, html.querySelectorAll("foreignObject").length]"##,
    );
    assert_eq!(out, [1, 1, 0, 1, 0, 1, 1, 1]);
}

#[test]
fn invalid_selectors_throw_a_syntax_error_from_the_page_realm() {
    let rt = runtime();
    for call in [
        r##"document.querySelector(":::")"##,
        r##"document.querySelectorAll(":has(:has(p))")"##,
        r##"document.body.matches("[")"##,
        r##"document.body.closest("p::totallyfake")"##,
    ] {
        let check: bool = rt.eval(&format!(
            "try {{ {call}; false }} catch (e) {{ e instanceof DOMException }}"
        ));
        assert!(check, "{call}");
        let (name, _, _) = js_error(rt.try_eval(call));
        assert_eq!(name, "SyntaxError", "{call}");
    }
}

#[test]
fn legal_but_unsupported_pseudos_parse_and_never_match() {
    let rt = runtime();
    let out: Vec<usize> = rt.eval(&format!(
        r##"{PAGE}
        [":visited", ":modal", ":popover-open", ":-webkit-autofill", "p::before", "::after",
            "p:before", "::backdrop", "::-webkit-scrollbar"]
            .map((s) => document.querySelectorAll(s).length)
            .concat(document.querySelectorAll("p:not(:visited)").length)"##
    ));
    assert_eq!(out, [0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);
}

#[test]
fn declined_selectors_keep_working() {
    let rt = runtime();
    let out: Vec<String> = rt.eval(&format!(
        r##"{PAGE}
        document.getElementById("i1").focus();
        const box = document.createElement("input");
        box.type = "checkbox";
        box.id = "cb";
        document.body.append(box);
        box.checked = true;
        [":lang(en)", "p:defined", "input:enabled", "input:checked", ":focus", "section:focus-within",
            "input:not(:checked)", "#root input:required"]
            .map((s) => ids(document.querySelectorAll(s)))"##
    ));
    assert_eq!(out, ["p1", "p1 p2", "i1 cb", "cb", "i1", "a", "i1", ""]);
}

/// Every selector in `CASES` must take the native path, not silently fall back.
#[test]
fn cases_match_natively_and_declined_selectors_fall_back() {
    let rt = runtime();
    let fallbacks: Vec<String> = rt.eval(&format!(
        r##"{PAGE}
        const wire = __jsdomImplForWrapper(document)._wire;
        const out = new Uint32Array(64);
        {selectors:?}.concat([":lang(en)", ":checked", ":hover", ":::"])
            .filter((s) => Deno.core.ops.op_dom_query(wire, s, 1, true, false, out) < 0)"##,
        selectors = CASES.iter().map(|(s, _)| *s).collect::<Vec<_>>()
    ));
    assert_eq!(fallbacks, [":lang(en)", ":checked", ":hover", ":::"]);
}

#[test]
fn id_selectors_respect_scope_duplicates_and_quirks() {
    let rt = runtime();
    let out: Vec<serde_json::Value> = rt.eval(
        r##"
        const tags = (list) => [...list].map((e) => e.localName + (e.className ? "." + e.className : ""));
        document.body.innerHTML = '<div id="w"><p id="x" class="c"></p></div><p id="x"></p><span id="Up"></span>';
        const w = document.getElementById("w");
        const lone = document.createElement("i");
        lone.id = "solo";
        const d = document.createElement("div");
        d.innerHTML = '<b id="k"></b>';
        const standards = new DOMParser().parseFromString('<!doctype html><p id="Up">', "text/html");
        [tags(document.querySelectorAll("#x")), tags(w.querySelectorAll("#x")),
            tags(document.querySelectorAll("p#x.c")), tags(document.querySelectorAll("div > #x")),
            tags(w.querySelectorAll("#w")), document.querySelector("#solo"),
            d.querySelector("#k").localName, document.compatMode,
            tags(document.querySelectorAll("#up")), standards.querySelectorAll("#up").length,
            standards.querySelectorAll("#Up").length]"##,
    );
    assert_eq!(
        serde_json::Value::Array(out).to_string(),
        r#"[["p.c","p"],["p.c"],["p.c"],["p.c"],[],null,"b","BackCompat",["span"],0,1]"#
    );
}
