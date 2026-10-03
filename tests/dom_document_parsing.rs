//! Document parsing (page loads through `document.write`, `DOMParser`): the parser builds the
//! tree in the arena and stops at each `<script>`, so a script sees only the document parsed
//! before it. Read back through the public API only.

mod common;

use common::{EvalExt, runtime};

/// Replaces the document with `html` the way a page load does, with `log` as a global array
/// the page's scripts push to.
fn load_js(html: &str) -> String {
    format!(
        r#"window.log = [];
        document.open();
        document.write({html:?});
        document.close();"#
    )
}

#[test]
fn inline_scripts_run_in_order_and_see_only_what_was_parsed_before_them() {
    let rt = runtime();
    let js = load_js(
        r#"<!DOCTYPE html><html><head>
        <script>log.push("head " + document.querySelectorAll("p").length + " " + !!document.body)</script>
        </head><body><p>1</p>
        <script>log.push("mid " + document.querySelectorAll("p").length + " " + document.getElementById("late"))</script>
        <p id=late>2</p>
        <script>log.push("end " + document.querySelectorAll("p").length + " " + document.getElementById("late").id)</script>
        </body></html>"#,
    );
    let log: Vec<String> = rt.eval(&format!("{js}\nlog"));
    assert_eq!(log, ["head 0 false", "mid 1 null", "end 2 late"]);
}

#[test]
fn the_running_script_is_the_last_node_parsed() {
    let rt = runtime();
    let js = load_js(
        r#"<body><div id=d>a<script>log.push(document.currentScript.parentNode.id, document.currentScript.nextSibling, document.getElementById("d").childNodes.length)</script>b</div></body>"#,
    );
    let log: serde_json::Value = rt.eval(&format!("{js}\nlog"));
    assert_eq!(log, serde_json::json!(["d", null, 2]));
}

#[test]
fn external_and_inline_scripts_run_in_document_order() {
    let rt = runtime();
    let js = load_js(
        r#"<body><script>log.push("first")</script>
        <script src="data:text/javascript,log.push('external')"></script>
        <p>x</p><script>log.push("inline after external")</script></body>"#,
    );
    let log: Vec<String> = rt.eval_promise(&format!(
        r#"const loaded = new Promise((resolve) => window.addEventListener("load", resolve, {{ once: true }}));
        {js}
        loaded.then(() => log)"#
    ));
    assert_eq!(log, ["first", "external", "inline after external"]);
}

#[test]
fn document_write_during_parse_inserts_after_the_running_script() {
    let rt = runtime();
    let js = load_js(
        r#"<body><p id=a></p><script>document.write("<p id=w>w</p><script>log.push('written ' + document.getElementById('w').id)</scr" + "ipt>")</script><p id=z></p><script>log.push([...document.body.children].map((e) => e.id || e.localName).join())</script></body>"#,
    );
    let log: Vec<String> = rt.eval(&format!("{js}\nlog"));
    assert_eq!(log, ["written w", "a,script,w,script,z,script"]);
}

#[test]
fn parsed_document_has_the_implied_structure_and_doctype() {
    let rt = runtime();
    let js = load_js("<!DOCTYPE html><title>t</title><p>x<table><tr><td>c</td></tr>foster</table>");
    let got: serde_json::Value = rt.eval(&format!(
        r#"{js}
        [document.doctype.name, document.compatMode, document.title, document.documentElement.outerHTML]"#
    ));
    assert_eq!(
        got,
        serde_json::json!([
            "html",
            "CSS1Compat",
            "t",
            "<html><head><title>t</title></head><body><p>x</p>foster<table><tbody><tr><td>c</td></tr></tbody></table></body></html>"
        ])
    );
    let js = load_js("<p>no doctype</p>");
    let got: serde_json::Value = rt.eval(&format!("{js}\n[document.doctype, document.compatMode]"));
    assert_eq!(got, serde_json::json!([null, "BackCompat"]));
}

#[test]
fn template_content_stays_out_of_the_main_tree() {
    let rt = runtime();
    let js = load_js(
        r#"<body><template id=t><p class=in>x</p><script>log.push("template script ran")</script></template><p class=out></p></body>"#,
    );
    let got: serde_json::Value = rt.eval(&format!(
        r#"{js}
        const t = document.getElementById("t");
        [document.querySelectorAll("p").length, t.childNodes.length,
         t.content.querySelector(".in").textContent, t.content.ownerDocument !== document,
         t.innerHTML, log]"#
    ));
    assert_eq!(
        got,
        serde_json::json!([
            1,
            0,
            "x",
            true,
            r#"<p class="in">x</p><script>log.push("template script ran")</script>"#,
            []
        ])
    );
}

#[test]
fn style_and_stylesheet_links_reach_document_style_sheets() {
    let rt = runtime();
    let js = load_js(
        r#"<head><style>p { color: red }</style>
        <script>log.push(document.styleSheets.length)</script>
        <link rel=stylesheet href="data:text/css,b{color:blue}"></head><body><p>x</p><b>y</b></body>"#,
    );
    let got: serde_json::Value = rt.eval_promise(&format!(
        r#"const loaded = new Promise((resolve) => window.addEventListener("load", resolve, {{ once: true }}));
        {js}
        loaded.then(() => [log[0], document.styleSheets.length,
            getComputedStyle(document.querySelector("p")).color,
            getComputedStyle(document.querySelector("b")).color])"#
    ));
    assert_eq!(
        got,
        serde_json::json!([1, 2, "rgb(255, 0, 0)", "rgb(0, 0, 255)"])
    );
}

#[test]
fn custom_elements_defined_earlier_construct_and_connect_during_parse() {
    let rt = runtime();
    let js = load_js(
        r#"<body><script>customElements.define("x-a", class extends HTMLElement {
            constructor() { super(); log.push("construct " + this.parentNode); }
            connectedCallback() { log.push("connected " + this.id + " " + this.isConnected); }
        })</script><x-a id=one></x-a><script>log.push("after")</script></body>"#,
    );
    let log: Vec<String> = rt.eval(&format!("{js}\nlog"));
    // jsdom runs the callback from the backup element queue, after the next script.
    assert_eq!(log, ["construct null", "after", "connected one true"]);
}

#[test]
fn mutation_observers_see_nodes_the_parser_inserts_later() {
    let rt = runtime();
    let js = load_js(
        r#"<body><script>window.mo = new MutationObserver(() => {});
        mo.observe(document.body, { childList: true, subtree: true })</script><p>a<b>b</b></p><script>
        log.push(...mo.takeRecords().map((r) => r.target.nodeName + ">" + [...r.addedNodes].map((n) => n.nodeName).join()))</script></body>"#,
    );
    let log: Vec<String> = rt.eval(&format!("{js}\nlog"));
    assert_eq!(
        log,
        [
            "BODY>P",
            "P>#text",
            "P>B",
            "B>#text",
            "BODY>SCRIPT",
            "SCRIPT>#text"
        ]
    );
}

#[test]
fn form_controls_get_their_state_during_parse() {
    let rt = runtime();
    let js = load_js(
        r#"<body><form id=f><input name=a value=1><select name=s><option>x<option selected>y</select>
        <textarea name=t>txt</textarea></form><input form=f name=outside></body>"#,
    );
    let got: serde_json::Value = rt.eval(&format!(
        r#"{js}
        const f = document.forms.f;
        [f.elements.length, f.elements.namedItem("s").value, f.elements.namedItem("t").value, [...new FormData(f).keys()]]"#
    ));
    assert_eq!(
        got,
        serde_json::json!([4, "y", "txt", ["a", "s", "t", "outside"]])
    );
}

#[test]
fn dom_parser_sets_the_html_or_xml_flag() {
    let rt = runtime();
    let got: serde_json::Value = rt.eval(
        r#"const html = new DOMParser().parseFromString("<div><SPAN></SPAN><script>window.ranInParsed = 1</script></div>", "text/html");
        const xml = new DOMParser().parseFromString("<root><Item/></root>", "application/xml");
        [html.querySelectorAll("span").length, html.querySelector("div").firstChild.tagName,
         xml.querySelectorAll("item").length, xml.querySelectorAll("Item").length,
         xml.documentElement.firstChild.tagName, window.ranInParsed ?? null]"#,
    );
    assert_eq!(got, serde_json::json!([1, "SPAN", 0, 1, "Item", null]));
}

#[test]
fn the_parser_keeps_filling_an_element_a_script_detached() {
    let rt = runtime();
    let js = load_js(
        r#"<body><div id=d><script>window.d = document.getElementById("d"); d.remove()</script><p>late</p>text</div><p id=after></p></body>"#,
    );
    let got: serde_json::Value = rt.eval(&format!(
        r#"{js}
        [d.isConnected, d.innerHTML.replace(/<script>.*<\/script>/, ""), d.lastChild.parentNode === d,
         document.body.innerHTML]"#
    ));
    assert_eq!(
        got,
        serde_json::json!([false, "<p>late</p>text", true, r#"<p id="after"></p>"#])
    );
}

#[test]
fn custom_elements_constructed_during_parse_see_their_attributes_change() {
    let rt = runtime();
    let js = load_js(
        r#"<body><script>customElements.define("x-b", class extends HTMLElement {
            static observedAttributes = ["foo"];
            attributeChangedCallback(name, old, value) { log.push(name + " " + old + " " + value); }
        })</script><x-b foo=1 bar=2></x-b></body>"#,
    );
    let log: Vec<String> = rt.eval(&format!("{js}\nlog"));
    assert_eq!(log, ["foo null 1"]);
}

#[test]
fn edits_to_nodes_scripts_already_saw_run_their_steps() {
    let rt = runtime();
    let js = load_js(
        r#"<body><p id=p>a<script>window.mo = new MutationObserver(() => {});
        mo.observe(document, { subtree: true, attributes: true, characterData: true });
        document.currentScript.remove()</script>b</p><body onclick="log.push('clicked')" data-x=1></body>"#,
    );
    let got: serde_json::Value = rt.eval(&format!(
        r#"{js}
        document.body.click();
        const p = document.getElementById("p");
        [log, document.body.dataset.x, p.childNodes.length, p.firstChild.data,
         mo.takeRecords().map((r) => r.type + " " + (r.attributeName ?? r.target.nodeName)).sort()]"#
    ));
    assert_eq!(
        got,
        serde_json::json!([
            ["clicked"],
            "1",
            1,
            "ab",
            [
                "attributes data-x",
                "attributes onclick",
                "characterData #text"
            ]
        ])
    );
}
