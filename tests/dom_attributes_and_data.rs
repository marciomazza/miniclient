//! Attributes and character data live in the native arena: every public write lands there,
//! and every public read agrees with what the arena holds.

mod common;

use common::{EvalExt, runtime};

/// The arena's view of a node, next to the public API's.
const ARENA: &str = r#"
    const wire = (n) => __jsdomImplForWrapper(n)._wire;
    const arenaAttrs = (el) => Deno.core.ops.op_dom_attrs(wire(el))
        .map((a) => [a.ns, a.prefix, a.local, a.value]);
    const publicAttrs = (el) => [...el.attributes]
        .map((a) => [a.namespaceURI, a.prefix, a.localName, a.value]);
    const arenaData = (n) => Deno.core.ops.op_dom_data(wire(n));
"#;

#[test]
fn attribute_edits_land_in_the_arena() {
    let rt = runtime();
    let out = rt.eval_json(&format!(
        r#"{ARENA}
        const el = document.createElement("div");
        document.body.append(el);
        el.setAttribute("data-x", "1");
        el.setAttribute("Title", "t");
        el.toggleAttribute("hidden");
        el.className = "a b";
        el.classList.add("c");
        el.classList.remove("a");
        el.id = "one";
        el.attributes["data-x"].value = "2";
        el.attributes.removeNamedItem("title");
        const extra = document.createAttribute("lang");
        extra.value = "pt";
        el.attributes.setNamedItem(extra);
        el.toggleAttribute("hidden");
        el.removeAttribute("missing");
        [publicAttrs(el), JSON.stringify(arenaAttrs(el)) === JSON.stringify(publicAttrs(el)),
            el.getAttribute("class"), el.getAttributeNames(), document.getElementById("one") === el,
            document.querySelector("[data-x='2'].c") === el, el.hasAttribute("hidden")]"#
    ));
    assert_eq!(
        out.as_deref(),
        Some(concat!(
            r#"[[[null,null,"data-x","2"],[null,null,"class","b c"],[null,null,"id","one"],"#,
            r#"[null,null,"lang","pt"]],true,"b c",["data-x","class","id","lang"],true,true,false]"#
        ))
    );
}

#[test]
fn attr_nodes_are_live_views_with_stable_identity() {
    let rt = runtime();
    let out = rt.eval_json(&format!(
        r#"{ARENA}
        const el = document.createElement("p");
        el.setAttribute("a", "1");
        const attr = el.getAttributeNode("a");
        el.setAttribute("a", "2");
        const live = attr.value;
        attr.value = "3";
        const same = el.attributes[0] === attr && el.attributes.a === attr;
        el.removeAttributeNode(attr);
        attr.value = "4";
        const other = document.createElement("i");
        other.setAttributeNode(attr);
        [live, el.getAttribute("a"), same, attr.ownerElement === other, el.attributes.length,
            other.getAttribute("a"), arenaAttrs(el), arenaAttrs(other)]"#
    ));
    assert_eq!(
        out.as_deref(),
        Some(r#"["2",null,true,true,0,"4",[],[[null,null,"a","4"]]]"#)
    );
}

#[test]
fn prefixes_and_namespaces_round_trip() {
    let rt = runtime();
    let out = rt.eval_json(&format!(
        r##"{ARENA}
        const XLINK = "http://www.w3.org/1999/xlink";
        const el = document.createElementNS("http://www.w3.org/2000/svg", "svg:use");
        el.setAttributeNS(XLINK, "xlink:href", "#a");
        el.setAttributeNS(XLINK, "other:href", "#b");
        el.setAttribute("href", "plain");
        const replaced = document.createAttributeNS(XLINK, "x:href");
        replaced.value = "#c";
        const old = el.setAttributeNodeNS(replaced);
        el.setAttributeNS("urn:n", "n:k", "v");
        el.removeAttributeNS("urn:n", "k");
        [publicAttrs(el), JSON.stringify(arenaAttrs(el)) === JSON.stringify(publicAttrs(el)),
            old.value, old.prefix, old.ownerElement, el.getAttributeNS(XLINK, "href"),
            el.getAttribute("x:href"), el.hasAttributeNS("urn:n", "k"),
            arenaAttrs(el.cloneNode())[0]]"##
    ));
    assert_eq!(
        out.as_deref(),
        Some(concat!(
            r##"[[["http://www.w3.org/1999/xlink","x","href","#c"],[null,null,"href","plain"]],"##,
            r##"true,"#b","xlink",null,"#c","#c",false,"##,
            r##"["http://www.w3.org/1999/xlink","x","href","#c"]]"##
        ))
    );
}

#[test]
fn character_data_edits_land_in_the_arena() {
    let rt = runtime();
    let out = rt.eval_json(&format!(
        r#"{ARENA}
        const p = document.createElement("p");
        document.body.append(p);
        const t = document.createTextNode("hello");
        const c = document.createComment("c");
        const pi = document.createProcessingInstruction("x", "dd");
        const piStart = [pi.data, wire(pi)];
        p.append(t, c);
        t.appendData(" world");
        t.nodeValue = t.data.toUpperCase();
        const tail = t.splitText(5);
        c.data = "cc";
        c.insertData(0, ">");
        pi.replaceData(0, 1, "pi");
        const cdata = new DOMParser().parseFromString("<r/>", "application/xml")
            .createCDATASection("cd");
        cdata.deleteData(0, 1);
        [piStart, [t, tail, c, pi, cdata].map(arenaData), [t, tail, c, pi, cdata].map((n) => n.data),
            p.textContent, p.innerHTML, tail.length, t.substringData(1, 2)]"#
    ));
    assert_eq!(
        out.as_deref(),
        Some(concat!(
            r#"[["dd",-1],["HELLO"," WORLD",">cc","pid","d"],["HELLO"," WORLD",">cc","pid","d"],"#,
            r#""HELLO WORLD","HELLO WORLD<!-->cc-->",6,"EL"]"#
        ))
    );
}

#[test]
fn attribute_and_data_changes_reach_observers_and_callbacks() {
    let rt = runtime();
    let out = rt.eval_json(
        r#"const calls = [];
        customElements.define("x-watch", class extends HTMLElement {
            static observedAttributes = ["x", "y"];
            attributeChangedCallback(...args) { calls.push(args); }
        });
        const el = document.createElement("x-watch");
        const t = document.createTextNode("a");
        el.append(t);
        document.body.append(el);
        const mo = new MutationObserver(() => {});
        mo.observe(el, { attributes: true, attributeOldValue: true, characterData: true,
            characterDataOldValue: true, subtree: true });
        el.setAttribute("x", "1");
        el.setAttribute("x", "2");
        el.setAttributeNS("urn:n", "n:y", "3");
        el.removeAttribute("x");
        el.getAttributeNode("n:y").value = "4";
        t.data = "b";
        t.appendData("c");
        [mo.takeRecords().map((r) => [r.type, r.attributeName, r.attributeNamespace, r.oldValue]),
            calls]"#,
    );
    assert_eq!(
        out.as_deref(),
        Some(concat!(
            r#"[[["attributes","x",null,null],["attributes","x",null,"1"],"#,
            r#"["attributes","y","urn:n",null],["attributes","x",null,"2"],"#,
            r#"["attributes","y","urn:n","3"],["characterData",null,null,"a"],"#,
            r#"["characterData",null,null,"b"]],"#,
            r#"[["x",null,"1",null],["x","1","2",null],["y",null,"3","urn:n"],"#,
            r#"["x","2",null,null],["y","3","4","urn:n"]]]"#
        ))
    );
}
