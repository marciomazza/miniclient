//! The pure-JS `FormData` implementation (formdata.js): which form controls it collects,
//! `URLSearchParams` serialisation, and in-place `set()`.

mod common;

use common::{EvalExt, Runtime, runtime};

type Pairs = Vec<(String, String)>;

/// Build a form from `html`, collect its FormData, return the `(name, value)` entries.
/// The case HTML only ever uses double quotes, so a single-quoted JS string is safe.
fn form_pairs(rt: &Runtime, html: &str) -> Pairs {
    let js = format!(
        r#"
          const wrap = document.createElement('div');
          wrap.innerHTML = '{html}';
          const form = wrap.querySelector('form');
          [...new FormData(form).entries()];
        "#
    );
    rt.eval::<Pairs>(&js)
}

/// Run `js` against a fresh FormData `fd`, return the resulting `(name, value)` entries.
fn set_pairs(rt: &Runtime, js: &str) -> Pairs {
    let js = format!(
        r#"
          const fd = new FormData();
          {js};
          [...fd.entries()];
        "#
    );
    rt.eval::<Pairs>(&js)
}

/// Compare collected `(name, value)` entries against `&str` literal pairs.
fn assert_pairs(actual: &[(String, String)], expected: &[(&str, &str)], msg: &str) {
    let actual: Vec<(&str, &str)> = actual
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    assert_eq!(actual, expected, "{msg}");
}

#[test]
fn collects_successful_controls() {
    let rt = runtime();
    for (html, expected) in &[
        (
            r#"<form><input name="x" type="text" value="hello"></form>"#,
            vec![("x", "hello")],
        ),
        (
            r#"<form><input name="x" type="text"></form>"#,
            vec![("x", "")],
        ),
        (
            r#"<form><input name="foo" value="bar"></form>"#,
            vec![("foo", "bar")],
        ),
        (
            r#"<form><textarea name="msg">hello</textarea></form>"#,
            vec![("msg", "hello")],
        ),
        (
            r#"<form><textarea name="x"></textarea></form>"#,
            vec![("x", "")],
        ),
        (
            r#"<form><select name="x"><option value="a">A</option><option value="b" selected>B</option></select></form>"#,
            vec![("x", "b")],
        ),
        (
            r#"<form><select name="x"><option value="a">A</option><option value="b">B</option></select></form>"#,
            vec![("x", "a")],
        ),
        (
            r#"<form><select name="x"><option>text-only</option></select></form>"#,
            vec![("x", "text-only")],
        ),
        (
            r#"<form><select name="items" multiple><option value="a" selected>A</option><option value="b" selected>B</option><option value="c">C</option></select></form>"#,
            vec![("items", "a"), ("items", "b")],
        ),
        (
            r#"<form><input type="checkbox" name="agree" value="yes" checked></form>"#,
            vec![("agree", "yes")],
        ),
        (
            r#"<form><input type="checkbox" name="ok" checked></form>"#,
            vec![("ok", "on")],
        ),
        (
            r#"<form><input type="checkbox" name="x" value="" checked></form>"#,
            vec![("x", "")],
        ),
        (
            r#"<form><input type="checkbox" name="hobby" value="read" checked><input type="checkbox" name="hobby" value="game" checked></form>"#,
            vec![("hobby", "read"), ("hobby", "game")],
        ),
        (
            r#"<form><input type="radio" name="x" value="" checked></form>"#,
            vec![("x", "")],
        ),
        (
            r#"<form><input type="radio" name="color" value="red" checked></form>"#,
            vec![("color", "red")],
        ),
        (
            r#"<form><input type="radio" name="size" value="s"><input type="radio" name="size" value="m" checked><input type="radio" name="size" value="l"></form>"#,
            vec![("size", "m")],
        ),
    ] {
        assert_pairs(&form_pairs(&rt, html), expected, html);
    }
}

#[test]
fn excludes_unsuccessful_controls() {
    let rt = runtime();
    for html in [
        r#"<form><input type="checkbox" name="agree" value="yes"></form>"#,
        r#"<form><input type="radio" name="color" value="red"></form>"#,
        r#"<form><input name="x" value="1" disabled></form>"#,
        r#"<form><input value="1"></form>"#,
        r#"<form><input type="submit" name="s" value="go"></form>"#,
        r#"<form><input type="button" name="b" value="go"></form>"#,
        r#"<form><input type="file" name="f"></form>"#,
        r#"<form><select name="x"></select></form>"#,
        r#"<form><select name="x" multiple><option value="a">A</option></select></form>"#,
    ] {
        assert!(form_pairs(&rt, html).is_empty(), "{html}");
    }
}

#[test]
fn urlsearchparams_from_formdata() {
    let rt = runtime();
    for (html, expected) in [
        (r#"<form><input name="x" value="hello"></form>"#, "x=hello"),
        (
            r#"<form><input name="a" value="1"><input name="b" value="2"></form>"#,
            "a=1&b=2",
        ),
        (
            r#"<form><input name="q" value="hello world"></form>"#,
            "q=hello+world",
        ),
        (
            r#"<form><input type="checkbox" name="c" value="x" checked><input type="checkbox" name="c" value="y" checked></form>"#,
            "c=x&c=y",
        ),
        (r#"<form><input name="x" value=""></form>"#, "x="),
    ] {
        let js = format!(
            r#"
              const wrap = document.createElement('div');
              wrap.innerHTML = '{html}';
              const form = wrap.querySelector('form');
              new URLSearchParams(new FormData(form)).toString();
            "#
        );
        assert_eq!(rt.eval::<String>(&js), expected, "{html}");
    }
}

#[test]
fn set_replaces_in_place() {
    let rt = runtime();
    for (js, expected) in &[
        (
            "fd.append('a', '1'); fd.append('b', '2'); fd.append('c', '3'); fd.set('a', 'X')",
            vec![("a", "X"), ("b", "2"), ("c", "3")],
        ),
        (
            "fd.append('a', '1'); fd.set('b', '2')",
            vec![("a", "1"), ("b", "2")],
        ),
        (
            "fd.append('a', '1'); fd.append('a', '2'); fd.append('b', '3'); fd.set('a', 'X')",
            vec![("a", "X"), ("b", "3")],
        ),
    ] {
        assert_pairs(&set_pairs(&rt, js), expected, js);
    }
}
