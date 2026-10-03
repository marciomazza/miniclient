//! `innerHTML`/`outerHTML` of HTML documents come from the arena's serializer, byte for byte
//! what parse5 produces through jsdom's tree adapter.

mod common;

use common::{EvalExt, Runtime, runtime};

/// Evaluates `setup` (which must leave the node in `node`) and returns its public `innerHTML`
/// or `outerHTML` next to the arena's serialization of the same node. The public read comes
/// first, as it gives a node without a slot one.
fn both(rt: &Runtime, setup: &str, outer: bool, scripting: bool) -> (String, String) {
    let prop = if outer { "outerHTML" } else { "innerHTML" };
    rt.eval(&format!(
        r#"let node;
        {setup}
        const html = node.{prop};
        const wire = __jsdomImplForWrapper(node)._wire;
        [html, Deno.core.ops.op_dom_serialize(wire, {outer}, {scripting})]"#
    ))
}

#[track_caller]
fn assert_serializes(setup: &str, outer: bool, expected: &str) {
    let rt = runtime();
    let (public, native) = both(&rt, setup, outer, true);
    assert_eq!(public, expected, "public API");
    assert_eq!(native, expected, "arena serializer");
}

#[test]
fn void_elements_have_no_end_tag_or_children() {
    assert_serializes(
        r#"node = document.createElement("div");
        node.innerHTML = '<br><img src="a"><input value=x><hr><wbr><p>t</p>';
        const br = node.querySelector("br");
        br.append("ignored");"#,
        false,
        r#"<br><img src="a"><input value="x"><hr><wbr><p>t</p>"#,
    );
}

#[test]
fn attribute_values_escape_quote_ampersand_and_nbsp_only() {
    assert_serializes(
        r#"node = document.createElement("div");
        node.setAttribute("title", 'a"b&c<d>e\u00a0f\'g');
        node.setAttribute("data-x", "");"#,
        true,
        r#"<div title="a&quot;b&amp;c<d>e&nbsp;f'g" data-x=""></div>"#,
    );
}

#[test]
fn raw_text_elements_keep_their_text_unescaped() {
    assert_serializes(
        r#"node = document.createElement("div");
        for (const tag of ["script", "style", "xmp", "iframe", "noembed", "noframes",
                "plaintext", "noscript", "textarea", "pre", "p"]) {
            const el = document.createElement(tag);
            el.append('<a href="x">&\u00a0"</a>');
            node.append(el);
        }"#,
        false,
        concat!(
            r#"<script><a href="x">&\u00a0"</a></script>"#,
            r#"<style><a href="x">&\u00a0"</a></style>"#,
            r#"<xmp><a href="x">&\u00a0"</a></xmp>"#,
            r#"<iframe><a href="x">&\u00a0"</a></iframe>"#,
            r#"<noembed><a href="x">&\u00a0"</a></noembed>"#,
            r#"<noframes><a href="x">&\u00a0"</a></noframes>"#,
            r#"<plaintext><a href="x">&\u00a0"</a></plaintext>"#,
            r#"<noscript><a href="x">&\u00a0"</a></noscript>"#,
            r#"<textarea>&lt;a href="x"&gt;&amp;&nbsp;"&lt;/a&gt;</textarea>"#,
            r#"<pre>&lt;a href="x"&gt;&amp;&nbsp;"&lt;/a&gt;</pre>"#,
            r#"<p>&lt;a href="x"&gt;&amp;&nbsp;"&lt;/a&gt;</p>"#,
        )
        .replace("\\u00a0", "\u{a0}")
        .as_str(),
    );
}

#[test]
fn noscript_text_is_escaped_when_scripting_is_disabled() {
    let rt = runtime();
    let (public, native) = both(
        &rt,
        r#"const doc = new DOMParser().parseFromString("<noscript></noscript>", "text/html");
        node = doc.querySelector("noscript");
        node.append("<b>&");"#,
        true,
        false,
    );
    assert_eq!(public, "<noscript>&lt;b&gt;&amp;</noscript>");
    assert_eq!(native, public);
}

#[test]
fn template_serializes_its_content_fragment() {
    assert_serializes(
        r#"node = document.createElement("div");
        node.innerHTML = '<template id="t"><b>&amp;</b><template><i>x</i></template></template>';
        const clone = node.firstChild.cloneNode(true);
        clone.content.append("<");
        node.append(clone);"#,
        false,
        concat!(
            r#"<template id="t"><b>&amp;</b><template><i>x</i></template></template>"#,
            r#"<template id="t"><b>&amp;</b><template><i>x</i></template>&lt;</template>"#,
        ),
    );
    assert_serializes(
        r#"const t = document.createElement("template");
        t.innerHTML = "<p>in</p>";
        node = t;"#,
        false,
        "<p>in</p>",
    );
}

#[test]
fn foreign_elements_keep_their_case_and_attributes() {
    assert_serializes(
        r#"node = document.createElement("div");
        node.innerHTML = '<svg viewBox="0 0 1 1" xlink:href="a" xml:lang="b" xmlns:x="c">'
            + '<foreignObject><p>x</p></foreignObject><clipPath/></svg>'
            + '<math><mi>x</mi><mtext><b>y</b></mtext></math>';"#,
        false,
        concat!(
            r#"<svg viewBox="0 0 1 1" xlink:href="a" xml:lang="b" xmlns:x="c">"#,
            r#"<foreignObject><p>x</p></foreignObject><clipPath></clipPath></svg>"#,
            r#"<math><mi>x</mi><mtext><b>y</b></mtext></math>"#,
        ),
    );
}

#[test]
fn prefixed_names_serialize_qualified() {
    assert_serializes(
        r#"node = document.createElement("div");
        const el = document.createElementNS("urn:x", "p:q");
        el.setAttributeNS("urn:y", "z:w", "v");
        el.setAttributeNS("http://www.w3.org/XML/1998/namespace", "xml:space", "s");
        el.setAttributeNS("urn:y", "w2", "v2");
        // Prefixed, an HTML-namespace element is neither void nor raw text.
        const br = document.createElementNS("http://www.w3.org/1999/xhtml", "h:br");
        br.append("<");
        const script = document.createElementNS("http://www.w3.org/1999/xhtml", "h:script");
        script.append("<");
        node.append(el, br, script, document.createElement("div", { is: "x-foo" }),
            document.createElement("p", { is: "" }));"#,
        false,
        concat!(
            r#"<p:q z:w="v" xml:space="s" w2="v2"></p:q><h:br>&lt;</h:br>"#,
            r#"<h:script>&lt;</h:script><div is="x-foo"></div><p></p>"#,
        ),
    );
}

#[test]
fn comments_doctype_and_dropped_nodes() {
    assert_serializes(
        r#"node = document.createElement("div");
        const cdata = new DOMParser().parseFromString("<r/>", "text/xml").createCDATASection("c");
        node.append(document.createComment(" a-->b "), document.createProcessingInstruction("t", "d"),
            document.adoptNode(cdata), "x");"#,
        false,
        "<!-- a-->b -->x",
    );
    // A document has no public `innerHTML`, so only the arena side is checked.
    let rt = runtime();
    let native: String = rt.eval(
        r#"const doc = new DOMParser().parseFromString(
            "<!DOCTYPE html><html><head><title>t</title></head><body>&amp;</body></html>",
            "text/html");
        doc.documentElement.before(doc.createComment("c"));
        Deno.core.ops.op_dom_serialize(__jsdomImplForWrapper(doc)._wire, false, false)"#,
    );
    assert_eq!(
        native,
        "<!DOCTYPE html><!--c--><html><head><title>t</title></head><body>&amp;</body></html>"
    );
}

#[test]
fn shadow_root_inner_html_comes_from_the_arena() {
    assert_serializes(
        r#"const host = document.createElement("div");
        node = host.attachShadow({ mode: "open" });
        node.innerHTML = "<slot></slot><b>s</b>";"#,
        false,
        "<slot></slot><b>s</b>",
    );
}

#[test]
fn xml_documents_keep_xml_serialization() {
    let rt = runtime();
    let out: (String, String) = rt.eval(
        r#"const doc = new DOMParser().parseFromString(
            '<r xmlns="urn:r"><a b="&lt;"/><br/><![CDATA[c]]><?p d?></r>', "application/xml");
        [doc.documentElement.outerHTML, doc.documentElement.innerHTML]"#,
    );
    assert_eq!(
        out,
        (
            r#"<r xmlns="urn:r"><a b="&lt;"/><br/><![CDATA[c]]><?p d?></r>"#.into(),
            r#"<a xmlns="urn:r" b="&lt;"/><br xmlns="urn:r"/><![CDATA[c]]><?p d?>"#.into(),
        )
    );
}
