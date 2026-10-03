//! `getElementById` reads the arena's id index and `window.<id>`/`window.<name>` a tracker fed
//! from the arena per connected or disconnected subtree; both follow moves and edits.

mod common;

use common::{EvalExt, runtime};

#[test]
fn first_in_document_order_wins_also_after_removing_it() {
    let rt = runtime();
    let got: Vec<serde_json::Value> = rt.eval(
        r#"const w = document.defaultView;
        const host = document.body.appendChild(document.createElement("div"));
        host.innerHTML = "<p id='d'>1</p><section><p id='d'>2</p></section><p id='d'>3</p>";
        const text = () => document.getElementById("d")?.textContent ?? null;
        const out = [text(), w.d.length];
        host.firstChild.remove();
        out.push(text(), w.d.length);
        host.firstChild.remove();
        out.push(text(), w.d.textContent);
        host.firstChild.remove();
        out.push(text(), typeof w.d);
        out"#,
    );
    assert_eq!(
        got,
        [
            "1".into(),
            3.into(),
            "2".into(),
            2.into(),
            "3".into(),
            "3".into(),
            serde_json::Value::Null,
            "undefined".into()
        ]
    );
}

#[test]
fn removed_and_detached_elements_are_not_found_until_reinserted() {
    let rt = runtime();
    let got: Vec<bool> = rt.eval(
        r#"const w = document.defaultView;
        const host = document.body.appendChild(document.createElement("div"));
        host.innerHTML = "<p><i id='deep'></i></p>";
        const p = host.firstChild;
        p.remove();
        const out = [document.getElementById("deep") === null, "deep" in w];
        host.append(p);
        out.push(document.getElementById("deep") !== null, w.deep.localName === "i");
        host.replaceChildren();
        out.push(document.getElementById("deep") === null);
        out"#,
    );
    assert_eq!(got, [true, false, true, true, true]);
}

#[test]
fn id_and_name_edits_update_the_lookups() {
    let rt = runtime();
    let got: Vec<serde_json::Value> = rt.eval(
        r#"const w = document.defaultView;
        const host = document.body.appendChild(document.createElement("div"));
        host.innerHTML = "<p id='a'></p><embed name='n'>";
        const p = host.firstChild, embed = host.lastChild;
        const by = (id) => document.getElementById(id)?.localName ?? null;
        const out = [by("a")];
        p.setAttribute("id", "b");
        out.push(by("a"), by("b"), w.b?.localName ?? null);
        p.removeAttribute("id");
        out.push(by("b"), typeof w.b);
        p.attributes.setNamedItem(Object.assign(document.createAttribute("id"), { value: "c" }));
        p.getAttributeNode("id").value = "d";
        out.push(by("c"), by("d"), w.d.localName);
        embed.setAttribute("name", "m");
        out.push(typeof w.n, w.m.localName);
        embed.removeAttribute("name");
        out.push(typeof w.m);
        out"#,
    );
    assert_eq!(
        got,
        [
            "p".into(),
            serde_json::Value::Null,
            "p".into(),
            "p".into(),
            serde_json::Value::Null,
            "undefined".into(),
            serde_json::Value::Null,
            "p".into(),
            "p".into(),
            "undefined".into(),
            "embed".into(),
            "undefined".into()
        ]
    );
}

#[test]
fn parsed_documents_and_template_content_have_their_own_ids() {
    let rt = runtime();
    let got: Vec<serde_json::Value> = rt.eval(
        r#"const parsed = new DOMParser().parseFromString(
            "<p id='in-parsed'></p><template><i id='in-template'></i></template>", "text/html");
        const host = document.body.appendChild(document.createElement("div"));
        host.innerHTML = "<template id='t'><i id='in-template'></i></template>";
        [
            parsed.getElementById("in-parsed")?.localName ?? null,
            document.getElementById("in-parsed"),
            parsed.getElementById("in-template"),
            document.getElementById("in-template"),
            document.getElementById("t").content.getElementById("in-template").localName,
        ]"#,
    );
    assert_eq!(
        got,
        [
            "p".into(),
            serde_json::Value::Null,
            serde_json::Value::Null,
            serde_json::Value::Null,
            "i".into()
        ]
    );
}
