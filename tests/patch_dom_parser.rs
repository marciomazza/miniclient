//! mini's DOMParser patch: `<template>` content parsed with the right table context, and
//! `insertAdjacentHTML` parsing context / node order / DOMException behaviour.

mod common;

use common::{EvalExt, json, runtime, text};

#[test]
fn regular_html_unchanged() {
    let rt = runtime();
    assert_eq!(
        text(rt.eval("new DOMParser().parseFromString('<div><p>hello</p></div>', 'text/html').body.innerHTML",
        )),
        "<div><p>hello</p></div>",
    );
}

#[test]
fn body_wrapping() {
    let rt = runtime();
    assert_eq!(
        text(rt.eval("new DOMParser().parseFromString('<body><p>hi</p></body>', 'text/html').documentElement.outerHTML",
        )),
        "<html><head></head><body><p>hi</p></body></html>",
    );
}

#[test]
fn template_table_tags_in_content() {
    let rt = runtime();
    for (html, want) in [
        (
            "<template><tr><td>x</td></tr></template>",
            "<tr><td>x</td></tr>",
        ),
        (
            "<table><template><tr><td>x</td></tr></template></table>",
            "<tr><td>x</td></tr>",
        ),
        (
            "<div><template><tr><td>x</td></tr></template></div>",
            "<tr><td>x</td></tr>",
        ),
        ("<template><td>x</td></template>", "<td>x</td>"),
        ("<template><th>x</th></template>", "<th>x</th>"),
        (
            "<template><thead><tr><th>x</th></tr></thead></template>",
            "<thead><tr><th>x</th></tr></thead>",
        ),
        (
            "<template><tbody><tr><td>x</td></tr></tbody></template>",
            "<tbody><tr><td>x</td></tr></tbody>",
        ),
        (
            "<template><tfoot><tr><td>x</td></tr></tfoot></template>",
            "<tfoot><tr><td>x</td></tr></tfoot>",
        ),
    ] {
        let src = format!(
            "new DOMParser().parseFromString('{html}', 'text/html').querySelector('template').innerHTML"
        );
        assert_eq!(text(rt.eval(&src)), want, "{html}");
    }
}

#[test]
fn template_with_non_table_content() {
    let rt = runtime();
    for (html, want) in [
        ("<template><p>hello</p></template>", "<p>hello</p>"),
        (
            "<template><ul><li>a</li><li>b</li></ul></template>",
            "<ul><li>a</li><li>b</li></ul>",
        ),
        ("<template></template>", ""),
    ] {
        let src = format!(
            "new DOMParser().parseFromString('{html}', 'text/html').querySelector('template').innerHTML"
        );
        assert_eq!(text(rt.eval(&src)), want, "{html}");
    }
}

#[test]
fn template_attributes_preserved() {
    let rt = runtime();
    for (html, want) in [
        (
            r#"<template id="tmpl1"><tr><td>x</td></tr></template>"#,
            r#"<template id="tmpl1"><tr><td>x</td></tr></template>"#,
        ),
        (
            r#"<template data-foo="bar"><p>x</p></template>"#,
            r#"<template data-foo="bar"><p>x</p></template>"#,
        ),
    ] {
        let src = format!("new DOMParser().parseFromString('{html}', 'text/html').body.innerHTML");
        assert_eq!(text(rt.eval(&src)), want, "{html}");
    }
}

#[test]
fn multiple_templates() {
    let rt = runtime();
    let src = r#"
        const html =
            '<template id="a"><tr><td>A</td></tr></template>' +
            '<template id="b"><tr><td>B</td></tr></template>';
        new DOMParser().parseFromString(html, 'text/html').body.innerHTML
    "#;
    assert_eq!(
        text(rt.eval(src)),
        r#"<template id="a"><tr><td>A</td></tr></template><template id="b"><tr><td>B</td></tr></template>"#,
    );
}

#[test]
fn nested_template_inner_content() {
    let rt = runtime();
    let src = r#"
        const html =
            '<template id="outer">' +
            '<template id="inner"><tr><td>deep</td></tr></template>' +
            '</template>';
        new DOMParser().parseFromString(html, 'text/html').querySelector('#outer').innerHTML
    "#;
    assert_eq!(
        text(rt.eval(src)),
        r#"<template id="inner"><tr><td>deep</td></tr></template>"#,
    );
}

#[test]
fn nested_template_outer_content_preserved() {
    let rt = runtime();
    let src = r#"
        const html =
            '<template id="outer">' +
            '<p>before</p>' +
            '<template id="inner"><p>inside</p></template>' +
            '<p>after</p>' +
            '</template>';
        new DOMParser().parseFromString(html, 'text/html').querySelector('#outer').innerHTML
    "#;
    assert_eq!(
        text(rt.eval(src)),
        r#"<p>before</p><template id="inner"><p>inside</p></template><p>after</p>"#,
    );
}

#[test]
fn insert_adjacent_html_table_context() {
    for (container, selector, markup, expected_selector) in [
        (
            "<table><tbody></tbody></table>",
            "tbody",
            "<tr><td>x</td></tr>",
            "tbody > tr",
        ),
        (
            "<table><tbody><tr></tr></tbody></table>",
            "tr",
            "<td>x</td>",
            "tr > td",
        ),
        (
            "<table></table>",
            "table",
            "<tbody><tr><td>x</td></tr></tbody>",
            "table > tbody",
        ),
    ] {
        let rt = runtime();
        rt.run(&format!(
                r#"document.body.innerHTML = "{container}";
                   document.querySelector("{selector}").insertAdjacentHTML("beforeend", "{markup}");"#
            ),
        );
        assert_eq!(
            json(rt.eval(&format!(
                r#"document.querySelectorAll("{expected_selector}").length"#
            ),))
            .as_deref(),
            Some("1"),
            "{container}",
        );
    }
}

#[test]
fn insert_adjacent_html_order() {
    for (position, expected) in [
        ("afterbegin", "<b>1</b><b>2</b><i>z</i>"),
        ("beforeend", "<i>z</i><b>1</b><b>2</b>"),
        ("beforebegin", "<b>1</b><b>2</b><div id='t'><i>z</i></div>"),
        ("afterend", "<div id='t'><i>z</i></div><b>1</b><b>2</b>"),
    ] {
        let rt = runtime();
        rt.run(&format!(
                r#"document.body.innerHTML = "<div id='t'><i>z</i></div>";
                   document.getElementById("t").insertAdjacentHTML("{position}", "<b>1</b><b>2</b>");"#
            ),
        );
        let target = if matches!(position, "afterbegin" | "beforeend") {
            "t"
        } else {
            "body"
        };
        let got = text(rt.eval(&format!(
                r#"({{t: () => document.getElementById("t"), body: () => document.body}})["{target}"]().innerHTML"#
            ),
        ));
        assert_eq!(got.replace('"', "'"), expected, "{position}");
    }
}

#[test]
fn insert_adjacent_html_throws_dom_exception() {
    // One shared runtime, so isolate the `const div` / `let out` bindings between iterations.
    let rt = runtime();
    for (position, want_error) in [
        ("beforebegin", "NoModificationAllowedError"),
        ("nonsense", "SyntaxError"),
    ] {
        let src = format!(
            r#"
            const div = document.createElement("div");
            let out;
            try {{
                div.insertAdjacentHTML("{position}", "<b>x</b>");
                out = ["no error", false, div.childNodes.length];
            }} catch (e) {{
                out = [e.name, e instanceof DOMException, div.childNodes.length];
            }}
            JSON.stringify(out)
        "#
        );
        assert_eq!(
            text(rt.eval(&src)),
            format!(r#"["{want_error}",true,0]"#),
            "{position}",
        );
    }
}
