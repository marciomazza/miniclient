//! `getComputedStyle`, answered by stylo, on hand-written pages and against a Chromium recording
//! of the CSS corpus. The cost test is a report: run it with `-- --ignored --nocapture`.

mod common;

use std::collections::BTreeMap;

use common::EvalExt;
use common::fetch_mock::{MockFetchBackend, runtime_with_mock};
use serde::Deserialize;

const HARNESS: &str = r#"
globalThis.__computed = (el, props) => {
    const cs = getComputedStyle(el);
    return props.map((p) => cs.getPropertyValue(p));
};
0
"#;

fn runtime() -> common::Runtime {
    let rt = runtime_with_mock(&MockFetchBackend::new());
    rt.run(HARNESS);
    rt
}

/// `getComputedStyle(document.querySelector(selector))` for each of `props`, after `markup`
/// replaces the document's content.
fn computed(rt: &common::Runtime, markup: &str, selector: &str, props: &[&str]) -> Vec<String> {
    let markup = serde_json::to_string(markup).unwrap();
    let args = serde_json::to_string(&(selector, props)).unwrap();
    rt.eval::<Vec<String>>(&format!(
        "document.documentElement.innerHTML = {markup}; \
         ((sel, props) => __computed(document.querySelector(sel), props))(...{args})"
    ))
}

const BASICS: &str = r#"<head><style>
        :root { --gap: 4px }
        .a { color: rgb(0, 0, 255); margin-top: var(--gap) }
        #b.a { color: green }
        p { display: inline-block !important }
        .c { & span { visibility: hidden } }
        @media (min-width: 600px) { .a { padding-left: 3px } }
        input:checked { outline-style: solid }
    </style></head>
    <body><div class="a"><p id="b" class="a" style="display: block">x</p>
    <div class="c"><span>y</span></div><input type="checkbox" checked></div></body>"#;

#[test]
fn cascade_basics() {
    let rt = runtime();
    let p = [
        "color",
        "display",
        "margin-top",
        "padding-left",
        "margin-block",
    ];
    assert_eq!(
        computed(&rt, BASICS, "#b", &p),
        ["rgb(0, 128, 0)", "inline-block", "4px", "3px", "4px 16px"]
    );
    assert_eq!(computed(&rt, BASICS, "span", &["visibility"]), ["hidden"]);
    assert_eq!(
        computed(&rt, BASICS, "input", &["outline-style"]),
        ["solid"]
    );
    // camelCase and `webkit*` accessors read the same values
    let camel: Vec<String> = rt.eval(
        r##"const cs = getComputedStyle(document.querySelector("#b"));
        [cs.marginTop, cs.webkitTransform, cs.getPropertyValue("-webkit-transform")]"##,
    );
    assert_eq!(camel, ["4px", "none", "none"]);
}

/// Queries and the cascade read element state from one place, so `matches` agrees with whether
/// a rule using the same selector applies.
#[test]
fn queries_and_the_cascade_agree_on_state_selectors() {
    let rt = runtime();
    let selectors = [
        ":default",
        ":checked",
        ":disabled",
        ":focus",
        ":focus-visible",
        ":read-write",
        ":required",
        ":any-link",
        ":lang(pt)",
        ":hover",
        ":valid",
        ":invalid",
        ":out-of-range",
        ":placeholder-shown",
        ":indeterminate",
    ];
    let out: Vec<(String, String, bool, bool)> = rt.eval(&format!(
        r#"document.documentElement.innerHTML = `<body><form lang="pt-BR">
            <input id="cb" type="checkbox" checked><input id="t" required>
            <button id="b">go</button><input id="d" disabled><a id="a" href="/x">x</a>
            <input id="n" type="number" max="1" value="5"><input id="p" placeholder="p">
            <input id="i" type="checkbox"></form></body>`;
        document.getElementById("i").indeterminate = true;
        document.getElementById("t").focus();
        const style = document.createElement("style");
        document.head.append(style);
        const out = [];
        for (const sel of {selectors:?}) {{
            style.textContent = `${{sel}} {{ outline-style: dotted }}`;
            for (const el of document.querySelectorAll("form *")) {{
                const applies = getComputedStyle(el).outlineStyle === "dotted";
                out.push([sel, el.id, el.matches(sel), applies]);
            }}
        }}
        out"#
    ));
    let disagree: Vec<_> = out.iter().filter(|(_, _, q, c)| q != c).collect();
    assert!(disagree.is_empty(), "{disagree:#?}");
    let matched = |sel: &str| -> Vec<&str> {
        out.iter()
            .filter(|(s, _, q, _)| s == sel && *q)
            .map(|(_, id, _, _)| id.as_str())
            .collect()
    };
    assert_eq!(matched(":default"), ["cb", "b"]);
    assert_eq!(matched(":focus-visible"), ["t"]);
    assert_eq!(matched(":lang(pt)").len(), 8);
    assert!(matched(":hover").is_empty());
    assert_eq!(matched(":out-of-range"), ["n"]);
    assert_eq!(matched(":indeterminate"), ["i"]);
}

/// Form state lives in jsdom, outside the document's change counter, so a new value alone
/// must restyle.
#[test]
fn a_value_change_restyles_form_state_rules() {
    let rt = runtime();
    let out: Vec<String> = rt.eval(
        r#"document.documentElement.innerHTML = `<head><style>
            input:placeholder-shown { color: rgb(1, 2, 3) }
            input:invalid { outline-style: solid }
        </style></head><body><input placeholder="p" required></body>`;
        const input = document.querySelector("input");
        const read = () => getComputedStyle(input).color + " " + getComputedStyle(input).outlineStyle;
        const before = read();
        input.value = "x";
        [before, read()]"#,
    );
    assert_eq!(out, ["rgb(1, 2, 3) solid", "rgb(0, 0, 0) none"]);
}

#[test]
fn inline_style_round_trips_through_the_attribute() {
    let rt = runtime();
    let got: serde_json::Value = rt.eval(
        r#"document.body.innerHTML = "<p style='margin: 1px; COLOR: red'>x</p>";
        const p = document.querySelector("p");
        const before = getComputedStyle(p).color;
        p.style.color = "rgb(10 20 30)";
        p.style.setProperty("padding-left", "2px", "important");
        p.style.marginTop = "3px";
        const written = p.getAttribute("style");
        const written_computed = getComputedStyle(p).color;
        p.setAttribute("style", "float: left; bogus: 1");
        ({
            before,
            written,
            written_computed,
            computed: getComputedStyle(p).color,
            names: [...p.style],
            cssFloat: p.style.cssFloat,
            removed: p.style.removeProperty("float"),
            after: p.getAttribute("style"),
        })"#,
    );
    assert_eq!(
        got,
        serde_json::json!({
            "before": "rgb(255, 0, 0)",
            "written": "margin: 3px 1px 1px; color: rgb(10, 20, 30); padding-left: 2px !important;",
            "written_computed": "rgb(10, 20, 30)",
            "computed": "rgb(0, 0, 0)",
            "names": ["float"],
            "cssFloat": "left",
            "removed": "left",
            "after": "",
        })
    );
    let computed: String = rt.eval(
        r#"const q = document.querySelector("p");
        q.style.cssText = "color: rgb(1, 2, 3)";
        getComputedStyle(q).color"#,
    );
    assert_eq!(computed, "rgb(1, 2, 3)");
}

#[test]
fn computed_style_enumerates_its_longhands() {
    let rt = runtime();
    let got: (u32, bool, bool, String) = rt.eval(
        r#"const cs = getComputedStyle(document.body);
        const names = [...cs];
        [cs.length, names.includes("color"), names.includes("margin"), cs.item(cs.length)]"#,
    );
    assert!(got.0 > 200, "{got:?}");
    assert!(got.1 && !got.2, "{got:?}");
    assert_eq!(got.3, "");
    let pseudo: u32 = rt.eval(r#"getComputedStyle(document.body, "::before").length"#);
    assert!(pseudo > 200);
    let unsupported: u32 = rt.eval(r#"getComputedStyle(document.body, "::unknown").length"#);
    assert_eq!(unsupported, 0);
    let edges: Vec<String> = rt.eval(
        r#"document.body.style.width = "7px";
        [getComputedStyle(document.body, "unknown").width,
            getComputedStyle(document.body, "::picker(select)").backgroundColor]"#,
    );
    assert_eq!(edges, ["7px", "rgba(0, 0, 0, 0)"]);
}

#[test]
fn pseudo_element_computed_style_uses_its_own_cascade() {
    let rt = runtime();
    let got: Vec<String> = rt.eval(
        r#"document.documentElement.innerHTML = `<head><style>
            p { color: rgb(0, 0, 255) }
            p::before { content: "x"; color: rgb(255, 0, 0) }
            li::marker { color: rgb(0, 128, 0) }
        </style></head><body><p>x</p><li>y</li></body>`;
        const p = document.querySelector("p");
        const before = getComputedStyle(p, "::before");
        [before.content, before.color, getComputedStyle(p).color,
            getComputedStyle(document.querySelector("li"), "::marker").color]"#,
    );
    assert_eq!(
        got,
        [
            r#""x""#,
            "rgb(255, 0, 0)",
            "rgb(0, 0, 255)",
            "rgb(0, 128, 0)"
        ]
    );
}

#[test]
fn pref_gated_features_parse() {
    let rt = runtime();
    let markup = r#"<style>
        #g { display: grid; grid-template-columns: 1px 2px; }
        #s { display: grid; grid-template-columns: subgrid; font-variation-settings: "wght" 700; }
    </style><div id="g"><div id="s"></div></div>"#;
    assert_eq!(
        computed(
            &rt,
            markup,
            "#s",
            &["grid-template-columns", "font-variation-settings"]
        ),
        ["subgrid", "\"wght\" 700"]
    );
}

#[test]
fn lang_matches_own_and_inherited_language() {
    let rt = runtime();
    let markup = r#"<style>
        :lang(en) { color: green; }
        :lang(fr) { color: blue; }
    </style>
    <p id="own" lang="EN-us"></p>
    <div lang="fr"><span id="inherited"></span><b id="override" lang="en"></b></div>
    <i id="eng" lang="eng"></i>"#;
    let color = |sel| computed(&rt, markup, sel, &["color"]).remove(0);
    assert_eq!(color("#own"), "rgb(0, 128, 0)");
    assert_eq!(color("#inherited"), "rgb(0, 0, 255)");
    assert_eq!(color("#override"), "rgb(0, 128, 0)");
    assert_eq!(color("#eng"), "rgb(0, 0, 0)");
}

#[test]
fn reflects_mutations() {
    let rt = runtime();
    let markup = "<head><style>.on { color: rgb(1, 2, 3) }</style></head><body><p>x</p></body>";
    assert_eq!(computed(&rt, markup, "p", &["color"]), ["rgb(0, 0, 0)"]);
    let color: String = rt.eval(
        r#"const p = document.querySelector("p"); p.className = "on"; getComputedStyle(p).color"#,
    );
    assert_eq!(color, "rgb(1, 2, 3)");
}

#[test]
fn cssom_writes_reach_the_cascade() {
    let rt = runtime();
    let markup = "<head><style>p { color: rgb(1, 2, 3) }</style></head><body><p>x</p></body>";
    assert_eq!(computed(&rt, markup, "p", &["color"]), ["rgb(1, 2, 3)"]);
    let colors: Vec<String> = rt.eval(
        r#"const sheet = document.styleSheets[0];
        const color = () => getComputedStyle(document.querySelector("p")).color;
        const seen = [];
        sheet.insertRule("p { color: rgb(4, 5, 6) }", 1);
        seen.push(color());
        sheet.cssRules[1].style.color = "rgb(7, 8, 9)";
        seen.push(color());
        sheet.disabled = true;
        seen.push(color());
        sheet.disabled = false;
        seen.push(color());
        const rule = sheet.cssRules[1];
        sheet.deleteRule(1);
        seen.push(color(), rule.cssText);
        seen"#,
    );
    assert_eq!(
        colors,
        [
            "rgb(4, 5, 6)",
            "rgb(7, 8, 9)",
            "rgb(0, 0, 0)",
            "rgb(7, 8, 9)",
            "rgb(1, 2, 3)",
            "p { color: rgb(7, 8, 9); }"
        ]
    );
}

#[test]
fn adopted_constructed_sheets_reach_the_cascade() {
    let rt = runtime();
    let got: serde_json::Value = rt.eval(
        r#"document.documentElement.innerHTML =
            "<head><style>p { color: rgb(1, 2, 3) }</style></head><body><p>x</p></body>";
        const color = () => getComputedStyle(document.querySelector("p")).color;
        const sheet = new CSSStyleSheet();
        sheet.replaceSync("p { color: rgb(4, 5, 6) }");
        const seen = [color()];
        document.adoptedStyleSheets = [sheet];
        seen.push(color());
        sheet.replaceSync("p { color: rgb(7, 8, 9) }");
        sheet.insertRule("p { color: rgb(10, 11, 12) }", sheet.cssRules.length);
        seen.push(color());
        sheet.deleteRule(1);
        sheet.disabled = true;
        seen.push(color());
        sheet.disabled = false;
        seen.push(color());
        document.adoptedStyleSheets.pop();
        seen.push(color());
        document.adoptedStyleSheets.push(sheet);
        seen.push(color());
        const errorName = value => {
            try {
                value.adoptedStyleSheets = [sheet];
                return "";
            } catch (error) {
                return error.name;
            }
        };
        let ownerError = "";
        try {
            document.adoptedStyleSheets = [document.styleSheets[0]];
        } catch (error) {
            ownerError = error.name;
        }
        let typeError = "";
        try {
            document.adoptedStyleSheets.push("not a sheet");
        } catch (error) {
            typeError = error.name;
        }
        let backing = null;
        Object.defineProperty(Array.prototype, "1", {
            configurable: true,
            set() { backing = this; },
        });
        try {
            document.adoptedStyleSheets = [sheet, new CSSStyleSheet()];
        } finally {
            delete Array.prototype["1"];
        }
        ({
            seen,
            ownerError,
            typeError,
            backingExposed: backing !== null,
            isArray: Array.isArray(document.adoptedStyleSheets),
            documentError: errorName(document.implementation.createHTMLDocument()),
        })"#,
    );
    assert_eq!(
        got,
        serde_json::json!({
            "seen": [
                "rgb(1, 2, 3)",
                "rgb(4, 5, 6)",
                "rgb(10, 11, 12)",
                "rgb(1, 2, 3)",
                "rgb(7, 8, 9)",
                "rgb(1, 2, 3)",
                "rgb(7, 8, 9)",
            ],
            "ownerError": "NotAllowedError",
            "typeError": "TypeError",
            "backingExposed": false,
            "isArray": true,
            "documentError": "NotAllowedError",
        })
    );
}

#[test]
fn hidden_ancestor_is_not_focusable() {
    let rt = runtime();
    let focused: Vec<bool> = rt.eval(
        r#"document.body.innerHTML =
            "<style>.off { display: none }</style><div class=off><button>a</button></div><button>b</button>";
        [...document.querySelectorAll("button")].map((b) => (b.focus(), document.activeElement === b))"#,
    );
    assert_eq!(focused, [false, true]);
}

#[test]
fn inner_text_of_a_hidden_element_is_empty() {
    let rt = runtime();
    let text: Vec<String> = rt.eval(
        r#"document.body.innerHTML = "<style>.off { display: none }</style><p class=off>a</p><p>b</p>";
        [...document.querySelectorAll("p")].map((p) => p.innerText)"#,
    );
    assert_eq!(text, ["", "b"]);
}

#[test]
fn outside_a_document_reads_empty() {
    let rt = runtime();
    let color: String = rt.eval(r#"getComputedStyle(document.createElement("p")).color"#);
    assert_eq!(color, "");
}

/// Writes `head` into the document over `mock`, waits for the window's `load`, then evaluates
/// `then`; `log` holds the `href` of each `<link>` that fired `error`.
fn after_load(mock: &MockFetchBackend, head: &str, then: &str) -> serde_json::Value {
    let rt = runtime_with_mock(mock);
    let html = format!(
        r#"<head><script>window.log = [];
        addEventListener("error", (e) => log.push(e.target.href), true)</script>{head}</head>
        <body><p>x</p></body>"#
    );
    rt.eval_promise(&format!(
        r#"const loaded = new Promise((resolve) => window.addEventListener("load", resolve, {{ once: true }}));
        document.open(); document.write({html:?}); document.close();
        loaded.then(() => {then})"#
    ))
}

const COLOR_OF_P: &str = r#"getComputedStyle(document.querySelector("p")).color"#;

#[test]
fn link_sheet_applies_and_goes_with_its_link() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/a.css", "p { color: rgb(1, 2, 3) }");
    let got = after_load(
        &mock,
        r#"<link rel=stylesheet href="http://localhost/a.css">"#,
        &format!(r#"[{COLOR_OF_P}, (document.querySelector("link").remove(), {COLOR_OF_P})]"#),
    );
    assert_eq!(got, serde_json::json!(["rgb(1, 2, 3)", "rgb(0, 0, 0)"]));
}

#[test]
fn rel_turning_into_stylesheet_loads_and_back_drops() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/a.css", "p { color: rgb(1, 2, 3) }");
    let got = after_load(
        &mock,
        r#"<link rel=preload as=style href="http://localhost/a.css">"#,
        &format!(
            r#"new Promise((resolve) => {{
                const link = document.querySelector("link");
                link.onload = resolve;
                link.rel = "stylesheet";
            }}).then(() => {{
                const loaded = [{COLOR_OF_P}, document.styleSheets.length];
                document.querySelector("link").rel = "preload";
                return [loaded, [{COLOR_OF_P}, document.styleSheets.length]];
            }})"#
        ),
    );
    assert_eq!(
        got,
        serde_json::json!([["rgb(1, 2, 3)", 1], ["rgb(0, 0, 0)", 0]])
    );
}

#[test]
fn failed_refetch_drops_the_previous_sheet() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/a.css", "p { color: rgb(1, 2, 3) }");
    mock.status("http://localhost/gone.css", 404);
    let got = after_load(
        &mock,
        r#"<link rel=stylesheet href="http://localhost/a.css">"#,
        &format!(
            r#"new Promise((resolve) => {{
                const link = document.querySelector("link");
                link.onerror = resolve;
                link.href = "http://localhost/gone.css";
            }}).then(() => [{COLOR_OF_P}, document.styleSheets.length])"#
        ),
    );
    assert_eq!(got, serde_json::json!(["rgb(0, 0, 0)", 0]));
}

#[test]
fn nested_import_resolves_against_its_own_sheet() {
    let mock = MockFetchBackend::new();
    mock.text(
        "http://localhost/a.css",
        r#"@import "css/b.css"; p { color: red }"#,
    );
    mock.text(
        "http://localhost/css/b.css",
        "@import url(c.css) screen; p { color: blue; background-color: green }",
    );
    mock.text(
        "http://localhost/css/c.css",
        "p { color: rgb(0, 0, 3); width: 1px }",
    );
    let got = after_load(
        &mock,
        r#"<link rel=stylesheet href="http://localhost/a.css">"#,
        r#"["color", "background-color", "width"].map(
            (p) => getComputedStyle(document.querySelector("p")).getPropertyValue(p))"#,
    );
    // Rules after an `@import` win over the imported ones.
    assert_eq!(
        got,
        serde_json::json!(["rgb(255, 0, 0)", "rgb(0, 128, 0)", "1px"])
    );
}

#[test]
fn stylo_discovers_escaped_import_keyword() {
    let mock = MockFetchBackend::new();
    mock.text(
        "http://localhost/a.css",
        r#"@i\6d port "b.css"; p { color: red }"#,
    );
    mock.text(
        "http://localhost/b.css",
        "p { color: rgb(1, 2, 3); width: 4px }",
    );
    let got = after_load(
        &mock,
        r#"<link rel=stylesheet href="http://localhost/a.css">"#,
        r#"["color", "width"].map(
            (p) => getComputedStyle(document.querySelector("p")).getPropertyValue(p))"#,
    );
    assert_eq!(got, serde_json::json!(["rgb(255, 0, 0)", "4px"]));
}

#[test]
fn imported_sheet_discovers_escaped_import_keyword() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/a.css", r#"@import "b.css";"#);
    mock.text(
        "http://localhost/b.css",
        r#"@\69 mport "c.css"; .md\:p { color: red }"#,
    );
    mock.text("http://localhost/c.css", "p { width: 4px }");
    let got = after_load(
        &mock,
        r#"<link rel=stylesheet href="http://localhost/a.css">"#,
        r#"getComputedStyle(document.querySelector("p")).width"#,
    );
    assert_eq!(got, serde_json::json!("4px"));
}

#[test]
fn failed_or_refused_import_adds_no_rules() {
    let mock = MockFetchBackend::new();
    mock.text(
        "http://localhost/a.css",
        r#"@import "gone.css"; @import "b.css" supports(not (display: block)); p { width: 1px }"#,
    );
    mock.status("http://localhost/gone.css", 404);
    mock.text("http://localhost/b.css", "p { color: red }");
    let got = after_load(
        &mock,
        r#"<link rel=stylesheet href="http://localhost/a.css">"#,
        r#"["color", "width"].map(
            (p) => getComputedStyle(document.querySelector("p")).getPropertyValue(p))"#,
    );
    assert_eq!(got, serde_json::json!(["rgb(0, 0, 0)", "1px"]));
}

#[test]
fn failed_fetch_fires_error_and_adds_no_rules() {
    let mock = MockFetchBackend::new();
    mock.status("http://localhost/gone.css", 404);
    let got = after_load(
        &mock,
        r#"<link rel=stylesheet href="http://localhost/gone.css">"#,
        &format!("[log, {COLOR_OF_P}]"),
    );
    assert_eq!(
        got,
        serde_json::json!([["http://localhost/gone.css"], "rgb(0, 0, 0)"])
    );
}

#[test]
fn media_gates_link_and_style_sheets() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/print.css", "p { color: red }");
    mock.text("http://localhost/wide.css", "p { width: 5px }");
    let got = after_load(
        &mock,
        r#"<link rel=stylesheet media=print href="http://localhost/print.css">
        <link rel=stylesheet media="screen and (min-width: 600px)" href="http://localhost/wide.css">
        <style media="(max-width: 100px)">p { height: 9px }</style>"#,
        r#"["color", "width", "height"].map(
            (p) => getComputedStyle(document.querySelector("p")).getPropertyValue(p))"#,
    );
    assert_eq!(got, serde_json::json!(["rgb(0, 0, 0)", "5px", "auto"]));
}

#[test]
fn sheets_a_browser_leaves_off_do_not_apply() {
    let mock = MockFetchBackend::new();
    mock.text("http://localhost/alt.css", "p { color: red }");
    mock.text("http://localhost/off.css", "p { width: 5px }");
    let got = after_load(
        &mock,
        r#"<link rel="alternate stylesheet" title=alt href="http://localhost/alt.css">
        <link rel=stylesheet disabled href="http://localhost/off.css">
        <style type="text/plain">p { height: 9px }</style>
        <style title=main>p { margin-top: 1px }</style>
        <style title=other>p { margin-left: 2px }</style>
        <style>p { padding-top: 3px }</style>"#,
        r#"["color", "width", "height", "margin-top", "margin-left", "padding-top"].map(
            (p) => getComputedStyle(document.querySelector("p")).getPropertyValue(p))"#,
    );
    assert_eq!(
        got,
        serde_json::json!(["rgb(0, 0, 0)", "auto", "auto", "1px", "0px", "3px"])
    );
}

const WPT: &str = "vendor/jsdom/test/web-platform-tests";

fn strip_scripts(html: &str) -> String {
    let re = regex::Regex::new(r"(?is)<script\b.*?</script\s*>").unwrap();
    re.replace_all(html, "").into_owned()
}

fn rss_kb() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

/// Run with `cargo test --release --test stylo_css cost -- --ignored --nocapture`.
#[test]
#[ignore = "report, not a gate"]
fn cost_report() {
    let rt = runtime();
    let rules: String = (0..50)
        .map(|i| {
            format!(
                ".c{i} {{ margin-left: {i}px }} .c{i} > .c{} span {{ color: red }}\n",
                i + 1
            )
        })
        .collect();
    let body: String = (0..500)
        .map(|i| format!("<div class=\"c{}\"><span>{i}</span></div>", i % 50))
        .collect();
    let markup = serde_json::to_string(&format!(
        "<head><style>{rules}</style></head><body>{body}</body>"
    ))
    .unwrap();
    rt.run(&format!("document.documentElement.innerHTML = {markup}; 0"));
    rt.run(
        r#"var els = [...document.querySelectorAll("span")], el = els[250], r = {};
        var time = (f) => { const t = performance.now(); f(); return performance.now() - t; };
        var avg = (n, f) => { let s = 0; for (let i = 0; i < n; i++) s += f(i); return s / n; };
        0"#,
    );
    let before = rss_kb();
    rt.run(r#"r.first = time(() => getComputedStyle(el).color); 0"#);
    let rss = rss_kb().saturating_sub(before);
    let json: String = rt.eval(
        r#"(() => {
            r.warm = avg(200, () => time(() => getComputedStyle(el).color));
            r.other = avg(200, (i) => time(() => getComputedStyle(els[i]).color));
            r.afterMutation = avg(20, (i) => {
                els[i].setAttribute("data-x", i);
                return time(() => getComputedStyle(el).color);
            });
            return JSON.stringify(r);
        })()"#,
    );
    println!("elements 1001, rules 100; times in ms: {json}");
    // jsdom's cascade on the same page; it no longer runs, so these numbers are fixed.
    println!(
        r#"jsdom's cascade was:            {{"first":7.81,"warm":0.0051,"afterMutation":0.750}}"#
    );
    println!("rss growth over the first call: +{rss} KiB");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Golden {
    chromium: String,
    viewport: (u32, u32),
    color_scheme: String,
    properties: Vec<String>,
    defaults: BTreeMap<String, String>,
    files: BTreeMap<String, Vec<RecordedElement>>,
}

/// An element's local name and the properties where Chromium's value differs from `defaults`.
type RecordedElement = (String, BTreeMap<String, String>);

/// Where jsdom's own cascade agreed with Chromium; that cascade no longer runs, so this cannot be
/// re-recorded. Per element, an index into `patterns`, each a hex bitset over
/// `Golden::properties`.
#[derive(Deserialize)]
struct JsdomVerdicts {
    patterns: Vec<String>,
    files: BTreeMap<String, Vec<usize>>,
}

impl JsdomVerdicts {
    fn agreed(&self, file: &str, element: usize, prop: usize) -> bool {
        let hex = &self.patterns[self.files[file][element]];
        let byte = u8::from_str_radix(&hex[prop / 8 * 2..prop / 8 * 2 + 2], 16).unwrap();
        byte & (1 << (prop % 8)) != 0
    }
}

/// `property [file]` lines; either ending in `*` is a prefix, no file means every file.
fn allow_list(text: &str) -> Vec<(String, Option<String>)> {
    text.lines()
        .map(|line| line.split('#').next().unwrap().trim())
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut words = line.split_whitespace();
            let prop = words.next().unwrap().to_owned();
            (prop, words.next().map(str::to_owned))
        })
        .collect()
}

/// Index of the first entry covering the pair.
fn allowed(allow: &[(String, Option<String>)], file: &str, prop: &str) -> Option<usize> {
    let matches = |pattern: &str, s: &str| match pattern.strip_suffix('*') {
        Some(prefix) => s.starts_with(prefix),
        None => s == pattern,
    };
    allow.iter().position(|(prop_pattern, file_pattern)| {
        matches(prop_pattern, prop) && file_pattern.as_deref().is_none_or(|f| matches(f, file))
    })
}

#[test]
fn allow_list_matching() {
    let allow = allow_list("# reason\nwidth\nscroll-snap-*  # gap\ncolor a.html\ndisplay d/*\n");
    assert!(allowed(&allow, "x.html", "width").is_some());
    assert!(allowed(&allow, "x.html", "min-width").is_none());
    assert!(allowed(&allow, "x.html", "scroll-snap-type").is_some());
    assert!(allowed(&allow, "a.html", "color").is_some());
    assert!(allowed(&allow, "b.html", "color").is_none());
    assert!(allowed(&allow, "d/x.html", "display").is_some());
    assert!(allowed(&allow, "e/x.html", "display").is_none());
}

/// Gate against Chromium's recorded values: a pair jsdom got right (`css_golden/jsdom.json`)
/// must stay right under stylo, unless `css_golden/allow.txt` lists it. Re-record with
/// `scripts/record-chromium-css.py`; the jsdom verdicts cannot be re-recorded, so a new file
/// needs its entry built some other way.
#[test]
fn chromium_golden() {
    let golden: Golden =
        serde_json::from_str(&std::fs::read_to_string("tests/css_golden/chromium.json").unwrap())
            .unwrap();
    let jsdom: JsdomVerdicts =
        serde_json::from_str(&std::fs::read_to_string("tests/css_golden/jsdom.json").unwrap())
            .unwrap();
    let allow = allow_list(&std::fs::read_to_string("tests/css_golden/allow.txt").unwrap());
    let rt = runtime();
    let window: (u32, u32) =
        serde_json::from_str(&rt.eval::<String>("JSON.stringify([innerWidth, innerHeight])"))
            .unwrap();
    // A recording on another device would mismatch media queries and viewport units.
    assert_eq!(
        golden.viewport, window,
        "recorded viewport differs from mini's window"
    );
    assert_eq!(
        golden.color_scheme, "light",
        "mini's device prefers the light scheme"
    );
    let props = serde_json::to_string(&golden.properties).unwrap();
    rt.run(&format!(
        r#"globalThis.__golden = (markup) => {{
            document.documentElement.innerHTML = markup;
            const props = {props};
            // Scripts and interactions are absent, and about:blank has no target element.
            const values = (el) => Deno.core.ops.op_css_computed_values(
                __jsdomImplForWrapper(el)._wire, -1, -1, props, document.baseURI, ""
            );
            return JSON.stringify(
                [...document.querySelectorAll("*")].map((el) => [el.localName, values(el)])
            );
        }};
        0"#
    ));
    let (mut pairs, mut stylo_eq, mut jsdom_eq, mut both) = (0, 0, 0, 0);
    let mut unread = Vec::new();
    let mut regressions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut used = vec![false; allow.len()];
    for (file, expected) in &golden.files {
        let html = std::fs::read_to_string(format!("{WPT}/{file}")).unwrap();
        let markup = serde_json::to_string(&strip_scripts(&html)).unwrap();
        let json = match rt.try_eval(&format!("__golden({markup})")) {
            Ok(Some(json)) => serde_json::from_str::<String>(&json).unwrap(),
            other => {
                unread.push(format!("{file}: {other:?}"));
                continue;
            }
        };
        let ours: Vec<(String, Vec<String>)> = serde_json::from_str(&json).unwrap();
        if ours
            .iter()
            .map(|element| &element.0)
            .ne(expected.iter().map(|element| &element.0))
        {
            unread.push(format!("{file}: element tree differs from Chromium's"));
            continue;
        }
        assert_eq!(
            jsdom.files[file].len(),
            expected.len(),
            "{file}: jsdom.json is stale"
        );
        for (n, ((_, stylo), (el, deltas))) in ours.iter().zip(expected).enumerate() {
            for (i, prop) in golden.properties.iter().enumerate() {
                let want = deltas.get(prop).unwrap_or(&golden.defaults[prop]);
                let (stylo_ok, jsdom_ok) = (&stylo[i] == want, jsdom.agreed(file, n, i));
                pairs += 1;
                stylo_eq += stylo_ok as usize;
                jsdom_eq += jsdom_ok as usize;
                both += (stylo_ok && jsdom_ok) as usize;
                if !jsdom_ok || stylo_ok {
                    continue;
                }
                if let Some(entry) = allowed(&allow, file, prop) {
                    used[entry] = true;
                } else {
                    let lost = regressions.entry(prop.clone()).or_default();
                    lost.push(format!(
                        "{file} <{el}>: chromium {want:?} stylo {:?}",
                        stylo[i]
                    ));
                }
            }
        }
    }
    let pct = |n: usize| 100.0 * n as f64 / pairs.max(1) as f64;
    println!(
        "Chromium {}: {} files ({} unread), {pairs} pairs",
        golden.chromium,
        golden.files.len(),
        unread.len()
    );
    println!("  stylo = Chromium {stylo_eq} ({:.1}%)", pct(stylo_eq));
    println!("  jsdom = Chromium {jsdom_eq} ({:.1}%)", pct(jsdom_eq));
    println!("  both             {both} ({:.1}%)", pct(both));
    for u in &unread {
        println!("  unread {u}");
    }
    for (prop, lost) in &regressions {
        println!("{:6} {prop}", lost.len());
        for sample in lost.iter().take(3) {
            println!("         {sample}");
        }
    }
    let stale: Vec<_> = allow
        .iter()
        .zip(&used)
        .filter(|(_, was_used)| !**was_used)
        .map(|(entry, _)| entry)
        .collect();
    for entry in &stale {
        println!("  stale allow entry {entry:?}");
    }
    assert!(unread.is_empty(), "{} files not compared", unread.len());
    // A fixed bug's entry would otherwise keep hiding a later regression.
    assert!(
        stale.is_empty(),
        "{} allow entries no longer cover a pair",
        stale.len()
    );
    assert!(
        regressions.is_empty(),
        "{} properties where stylo loses a value jsdom gets right",
        regressions.len()
    );
}

#[test]
fn css_supports_answers_from_stylo() {
    let rt = runtime();
    let cases = [
        (r#"CSS.supports("COLOR", "red")"#, true),
        (r#"CSS.supports("color", "nonsense")"#, false),
        (r#"CSS.supports("color", "red !important")"#, false),
        (r#"CSS.supports("not-a-prop", "red")"#, false),
        (r#"CSS.supports("grid-template-columns", "subgrid")"#, true),
        (r#"CSS.supports("--x", "anything")"#, true),
        (r#"CSS.supports("--a b", "x")"#, true),
        (r#"CSS.supports("color: red")"#, true),
        (
            r#"CSS.supports("(color: red) and (not (color: nonsense))")"#,
            true,
        ),
        (
            r#"CSS.supports("(color: nonsense) or (display: grid)")"#,
            true,
        ),
        (r#"CSS.supports("color: nonsense")"#, false),
        (r#"CSS.supports("(color: red) junk")"#, false),
    ];
    for (expr, expected) in cases {
        let got: bool = rt.eval(expr);
        assert_eq!(got, expected, "{expr}");
    }
}
