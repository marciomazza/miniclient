//! jsdom's tree structure lives in the native arena: every structural read jsdom makes goes
//! through `op_dom_*`, and node identity survives the round trip through wire ids.

mod common;

use common::{EvalExt, runtime};

/// Children of `el` as the arena sees them, mapped back to wire ids on the impls.
const ARENA_KIDS: &str = r#"
    const wire = (n) => __jsdomImplForWrapper(n)._wire;
    const arenaKids = (n) => Deno.core.ops.op_dom_children(wire(n)).map((r) => r.id);
"#;

#[test]
fn dom_mutations_land_in_the_arena() {
    let rt = runtime();
    let ok: bool = rt.eval(&format!(
        r#"{ARENA_KIDS}
        const div = document.createElement("div");
        const a = document.createElement("a");
        const t = document.createTextNode("x");
        div.append(a, t);
        div.insertBefore(document.createComment("c"), a);
        div.removeChild(t);
        document.body.append(div);
        JSON.stringify(arenaKids(div)) === JSON.stringify([...div.childNodes].map(wire))
            && arenaKids(document.body).includes(wire(div))
            && div.childNodes.length === 2"#
    ));
    assert!(ok);
}

#[test]
fn identity_is_stable_right_after_boot() {
    let rt = runtime();
    let ok: bool = rt.eval(
        r#"const div = document.createElement("div");
        const span = document.createElement("span");
        div.append(span, "t");
        document.body.append(div);
        const seen = new WeakMap([[span, 1]]);
        div.firstChild === span && span.parentNode === div && div.parentNode === document.body
            && seen.get(div.firstElementChild) === 1 && span.nextSibling.parentNode === div
            && document.body.lastChild === div"#,
    );
    assert!(ok);
}

#[test]
fn detached_roots_and_fragments_keep_their_structure() {
    let rt = runtime();
    let out = rt.eval_json(
        r#"const frag = document.createDocumentFragment();
        frag.append(document.createElement("p"), "x", document.createElement("i"));
        const loose = document.createElement("div");
        loose.innerHTML = "<b>1</b><b>2</b>";
        const t = document.createElement("template");
        t.innerHTML = "<em>e</em>";
        const before = [frag.childNodes.length, frag.lastChild.tagName, loose.textContent,
            loose.getRootNode() === loose, t.content.firstChild.tagName, t.childNodes.length];
        document.body.append(frag, loose, t);
        [...before, frag.childNodes.length, document.body.children.length,
            loose.getRootNode() === document, t.content.querySelectorAll("em").length]"#,
    );
    assert_eq!(
        out.as_deref(),
        Some(r#"[3,"I","12",true,"EM",0,0,4,true,1]"#)
    );
}

#[test]
fn tree_order_reads_match_the_dom() {
    let rt = runtime();
    let out = rt.eval_json(
        r#"document.body.innerHTML = "<div id=a><p id=b>x<i id=c></i></p><p id=d></p></div>";
        const $ = (id) => document.getElementById(id);
        const walker = document.createNodeIterator(document.body, NodeFilter.SHOW_ELEMENT);
        const fwd = [];
        for (let n; (n = walker.nextNode()); ) fwd.push(n.id || n.tagName);
        const back = [];
        for (let n; (n = walker.previousNode()); ) back.push(n.id || n.tagName);
        const range = document.createRange();
        range.setStart($("b"), 0);
        range.setEnd($("d"), 0);
        const loose = document.createElement("q");
        [fwd, back, $("b").compareDocumentPosition($("c")), $("d").compareDocumentPosition($("b")),
            $("b").compareDocumentPosition(loose) & Node.DOCUMENT_POSITION_DISCONNECTED,
            range.isPointInRange($("c"), 0), range.comparePoint($("a"), 0),
            [...$("a").children].map((e) => e.id), $("a").textContent, $("d").previousElementSibling.id,
            $("a").contains($("c")), Array.prototype.indexOf.call($("b").childNodes, $("c"))]"#,
    );
    assert_eq!(
        out.as_deref(),
        Some(
            r#"[["BODY","a","b","c","d"],["d","c","b","a","BODY"],20,2,1,true,-1,["b","d"],"x","b",true,1]"#
        )
    );
}

#[test]
fn an_attr_is_its_own_root() {
    let rt = runtime();
    let ok: bool = rt.eval(
        r#"const attr = document.createAttribute("x");
        attr.getRootNode() === attr && attr.parentNode === null"#,
    );
    assert!(ok);
}
