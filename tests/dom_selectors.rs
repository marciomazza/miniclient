//! `querySelector`, `querySelectorAll`, `closest` and `matches` through the public API. Selectors
//! parse as stylo parses them and match natively over the arena; state jsdom alone has (validity,
//! `:placeholder-shown`, ...) reaches the matcher as a snapshot taken per query.

mod common;

use std::collections::HashMap;

use common::{EvalExt, Runtime, js_error, runtime};

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
        r##"document.querySelectorAll("div,")"##,
    ] {
        let check: bool = rt.eval(&format!(
            "try {{ {call}; false }} catch (e) {{ e instanceof DOMException }}"
        ));
        assert!(check, "{call}");
        let (name, _, _) = js_error(rt.try_eval(call));
        assert_eq!(name, "SyntaxError", "{call}");
    }
}

/// Pseudo-classes stylo's parser does not know throw from every selector API, as in a browser
/// without them.
#[test]
fn pseudo_classes_stylo_does_not_know_throw() {
    let rt = runtime();
    for name in [
        "blank",
        "buffering",
        "current",
        "future",
        "has-slotted",
        "heading",
        "muted",
        "past",
        "paused",
        "picture-in-picture",
        "playing",
        "seeking",
        "stalled",
        "volume-locked",
    ] {
        for call in [
            format!(r#"document.querySelectorAll(":{name}").length"#),
            format!(r#"document.body.matches("p:{name}")"#),
            format!(r#"document.querySelector(":{name}")"#),
            format!(r#"document.body.closest(":{name}")"#),
        ] {
            let (error, _, _) = js_error(rt.try_eval(&call));
            assert_eq!(error, "SyntaxError", "{call}");
        }
    }
}

/// Names stylo lacks but browsers accept: pseudo-elements and `:-webkit-*` never match a query,
/// `:dir()` follows the `dir` attributes.
#[test]
fn names_stylo_lacks_match_as_in_a_browser() {
    let rt = runtime();
    let out: Vec<usize> = rt.eval(&format!(
        r##"{PAGE}
        document.getElementById("b").dir = "RTL";
        document.getElementById("e2").dir = "ltr";
        document.getElementById("e").dir = "auto";
        ["p::first-line", "p::first-letter", "::cue", "::cue(b)", "::target-text", "::cue-region",
            ":-webkit-autofill", "p:not(:-webkit-autofill)", "#root :dir(ltr)", "#root :dir(rtl)",
            "p:not(:dir(ltr))", ":is(span, i):dir(rtl)", "div:DIR(foo)", "[x=':in-range']"]
            .map((s) => document.querySelectorAll(s).length)"##
    ));
    assert_eq!(out, [0, 0, 0, 0, 0, 0, 0, 2, 10, 2, 0, 1, 0, 0]);
}

#[test]
fn escapes_dash_match_selector_lists_and_unclosed_blocks() {
    let rt = runtime();
    let out: Vec<String> = rt.eval(
        r##"document.body.innerHTML = `<div id="r">
            <i id="c1" class="before:after"></i><i id="c2" class="class1-x"></i>
            <i id="c3" class="x class1-y"></i><b id="b1"></b><b id="b2" type="x"></b></div>`;
        const ids = (s) => [...document.querySelectorAll(s)].map((e) => e.id).join(" ");
        // `|=` is equal-or-prefix on the whole value, not on a token; CSS closes open blocks
        // at the end of input, so a missing `)` is no error.
        [ids(".before\\:after"), ids('[class|="class1"]'), ids("#r :nth-last-child(2 of b)"),
            ids("#r b:not([type]")]"##,
    );
    assert_eq!(out, ["c1", "c2", "b1", "b1"]);
}

#[test]
fn legal_but_unsupported_pseudos_parse_and_never_match() {
    let rt = runtime();
    let out: Vec<usize> = rt.eval(&format!(
        r##"{PAGE}
        [":visited", ":modal", ":popover-open", ":hover", "p::before", "::after", "p:before",
            "::backdrop"]
            .map((s) => document.querySelectorAll(s).length)
            .concat(document.querySelectorAll("p:not(:visited)").length)"##
    ));
    assert_eq!(out, [0, 0, 0, 0, 0, 0, 0, 0, 2]);
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

/// Every selector in `CASES`, and the ones `dom-selector` used to answer, takes the native path:
/// what stylo rejects is invalid (`-2`), and one reading jsdom's form state asks for it (`-1`).
#[test]
fn cases_match_natively_and_rejected_selectors_are_invalid() {
    let rt = runtime();
    let declined: Vec<(String, i32)> = rt.eval(&format!(
        r##"{PAGE}
        const wire = __jsdomImplForWrapper(document)._wire;
        const out = new Uint32Array(64);
        const none = new Uint32Array(0);
        {selectors:?}.concat([":lang(en)", ":hover", ":dir(ltr)", "svg|rect", ":-webkit-autofill",
            "::first-line", ":invalid", ":::", ":-webkit-any(p)"])
            .map((s) => [s,
                Deno.core.ops.op_dom_query(wire, s, 1, true, false, -1, -1, false, none, out)])
            .filter(([, n]) => n < 0)"##,
        selectors = CASES.iter().map(|(s, _)| *s).collect::<Vec<_>>()
    ));
    // -1 asks JS for the form-state snapshot; -2 throws.
    let want = [
        ("svg|rect", -2),
        (":invalid", -1),
        (":::", -2),
        (":-webkit-any(p)", -2),
    ];
    assert_eq!(declined, want.map(|(s, n)| (s.to_owned(), n)));
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

const FORM_PAGE: &str = r##"
    document.body.innerHTML = `<form id="f">
      <fieldset id="fs" disabled><legend id="lg"><input id="inleg"></legend><input id="infs"></fieldset>
      <input id="t" placeholder="name" required>
      <input id="n" type="number" min="1" max="5" value="3">
      <input id="cb" type="CHECKBOX">
      <input id="r1" type="radio" name="g"><input id="r2" type="radio" name="g">
      <select id="sel"><option id="o1">a</option><option id="o2" selected>b</option>
        <optgroup id="og" disabled><option id="o3">c</option></optgroup></select>
      <textarea id="ta" readonly></textarea>
      <button id="sb">go</button><button id="b2" type="reset">r</button>
      <progress id="pg"></progress>
    </form>
    <div id="ce" contenteditable><span id="ces"></span></div>
    <a id="lk" href="/l">l</a><a id="nolk">n</a>
    <p id="x">target</p>`;
    const ids = (list) => [...list].map((el) => el.id || el.localName).join(" ");
    const $ = (id) => document.getElementById(id);
"##;

/// `(state change, selector, ids of document.querySelectorAll(selector))`, each case on a fresh
/// `FORM_PAGE` with no focus and no fragment.
const STATE_CASES: &[(&str, &str, &str)] = &[
    ("", "#f :checked", "o2"),
    ("$('r2').click()", "#f:has(:checked[type=radio])", "f"),
    ("$('sel').value = 'a'", "option:checked", "o1"),
    ("$('cb').checked = true", "#f :checked", "cb o2"),
    (
        "$('cb').checked = true",
        ":is(#cb, #r1):not(:checked)",
        "r1",
    ),
    ("", ":disabled", "fs infs og o3"),
    ("$('fs').disabled = false", "#fs :disabled", ""),
    ("", "#fs :enabled", "inleg"),
    ("", "fieldset:has(:enabled)", "fs"),
    ("", ":required", "t"),
    (
        "$('t').required = false",
        ":is(:required, #t:optional)",
        "t",
    ),
    ("", "#sel:optional", "sel"),
    ("", ":read-write", "inleg t n ce ces"),
    (
        "$('ta').readOnly = false",
        ":read-write",
        "inleg t n ta ce ces",
    ),
    ("", "#cb:read-only", "cb"),
    ("", ":placeholder-shown", "t"),
    ("$('t').value = 'x'", ":placeholder-shown", ""),
    ("", ":default", "o2 sb"),
    ("$('cb').defaultChecked = true", ":default", "cb o2 sb"),
    ("$('sb').type = 'reset'", ":default", "o2"),
    ("", ":indeterminate", "r1 r2 pg"),
    (
        "$('cb').indeterminate = true; $('r2').checked = true",
        ":indeterminate",
        "cb pg",
    ),
    ("", ":invalid", "f t"),
    ("$('t').value = 'x'", ":invalid", ""),
    ("$('t').value = 'x'", "form:valid", "f"),
    ("", ":in-range", "n"),
    ("$('n').value = '9'", ":out-of-range", "n"),
    ("$('n').value = '9'", ":in-range", ""),
    ("", ":focus", ""),
    ("$('t').focus()", ":focus", "t"),
    ("$('t').focus()", ":focus-within", "html body f t"),
    ("$('inleg').focus()", "fieldset:focus-within", "fs"),
    ("$('t').focus(); $('t').blur()", ":focus", ""),
    ("$('t').focus()", ":focus-visible", "t"),
    ("", ":link", "lk"),
    ("$('nolk').href = '/y'", ":any-link", "lk nolk"),
    ("", ":target", ""),
    ("location.hash = 'x'", ":target", "x"),
    (
        "location.hash = 'x'",
        "body:has(:target) :is(:link, :checked)",
        "o2 lk",
    ),
    ("$('x').id = '%41'; location.hash = '%41'", ":target", "%41"),
    ("$('fs').disabled = false", "#fs :enabled", "inleg infs"),
    ("$('ta').readOnly = false", "#ta:read-only", ""),
    ("$('lk').removeAttribute('href')", ":link", ""),
    ("", "#f :not(:disabled):is(select)", "sel"),
    (
        "$('t').value = 'x'",
        "#f:not(:has(:placeholder-shown))",
        "f",
    ),
    ("", "form:has(:default:is(option))", "f"),
    (
        "$('n').value = '9'",
        "#f :is(:in-range, :out-of-range)",
        "n",
    ),
    ("$('t').focus()", "#f:has(:focus-visible)", "f"),
    ("", "#f > :valid:not(button)", "fs n cb r1 r2 sel"),
    (
        "$('n').setAttribute('min', 'abc'); $('n').removeAttribute('max')",
        ":in-range",
        "",
    ),
    ("$('cb').setCustomValidity('no')", ":invalid", "f t cb"),
    (
        "$('t').value = 'x'; $('cb').required = true",
        "fieldset:valid, form:invalid",
        "f fs",
    ),
    ("$('n').step = '5'", ":invalid", "f t n"),
    (
        "document.body.append(Object.assign(document.createElement('form'), {id: 'g'})); \
         $('t').setAttribute('form', 'g')",
        "form:invalid",
        "g",
    ),
    (
        "const w = document.createElement('fieldset'); w.id = 'w'; \
         $('f').before(w); w.append($('f'))",
        "#w:invalid",
        "w",
    ),
    ("$('fs').disabled = false", "#fs :placeholder-shown", ""),
];

/// Also makes any query `dom-selector` would answer throw.
const RESET: &str = r##"
    __jsdomImplForWrapper(document)._getDOMSelector = () => { throw new Error("dom-selector"); };
    document.activeElement?.blur?.();
    history.replaceState(null, "", "/");
"##;

fn live_state_ids(rt: &Runtime, change: &str, selector: &str) -> String {
    rt.eval(&format!(
        r##"{RESET}{FORM_PAGE}{change};
        ids(document.querySelectorAll({selector:?}))"##
    ))
}

#[test]
fn state_pseudo_classes_follow_live_state() {
    let rt = runtime();
    let wrong: Vec<String> = STATE_CASES
        .iter()
        .filter_map(|(change, selector, want)| {
            let got = live_state_ids(&rt, change, selector);
            (got != *want).then(|| format!("{change} / {selector}: want {want:?}, got {got:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn state_pseudo_classes_in_closest_and_matches() {
    let rt = runtime();
    let out: Vec<serde_json::Value> = rt.eval(&format!(
        r##"{RESET}{FORM_PAGE}
        const before = [$("cb").matches(":checked"), $("ces").closest(":read-write").id];
        $("cb").checked = true;
        $("t").focus();
        [...before, $("cb").matches(":checked"), $("cb").matches(":not(:checked)"),
            $("t").closest(":invalid:not(input)").id, $("t").matches(":focus:placeholder-shown"),
            $("lg").closest(":has(:focus)").id, $("t").closest("form:focus-within").id]"##
    ));
    assert_eq!(
        serde_json::Value::Array(out).to_string(),
        r##"[false,"ces",true,false,"f",true,"f","f"]"##
    );
}

/// State set before an element joins a tree, or copied by cloning, is what `:checked` sees.
#[test]
fn checked_state_set_while_detached_is_matched_once_inserted() {
    let rt = runtime();
    let out: String = rt.eval(&format!(
        r##"{RESET}{FORM_PAGE}
        const box = document.createElement("input");
        box.type = "checkbox";
        box.id = "new";
        box.checked = true;
        const option = new Option("z", "z", false, true);
        option.id = "o9";
        $("sel").append(option);
        $("f").append(box, Object.assign(box.cloneNode(), {{ id: "copy" }}));
        ids(document.querySelectorAll(":checked"))"##
    ));
    assert_eq!(out, "o9 new copy");
}

/// Elements never inserted anywhere have no arena slot until a query gives them one.
#[test]
fn queries_on_a_never_inserted_element() {
    let rt = runtime();
    let out: Vec<serde_json::Value> = rt.eval(&format!(
        r##"{RESET}
        const p = document.createElement("p");
        [p.matches("p"), p.closest("p") === p, p.querySelector("b"),
            p.querySelectorAll("*").length, document.createElement("input").matches(":valid")]"##
    ));
    assert_eq!(
        serde_json::Value::Array(out).to_string(),
        "[true,true,null,0,true]"
    );
}
