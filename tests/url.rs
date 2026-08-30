//! `URL` / `URLSearchParams`: `searchParams` mutations propagate to `.search` / `.href`,
//! iterable constructor init, and `createObjectURL` / `revokeObjectURL`.

mod common;

use common::{boolean, eval, runtime, text};

#[test]
fn searchparams_mutation_propagates_to_search() {
    let rt = runtime();
    for (start_url, mutation, want) in [
        ("http://ex.com/", "u.searchParams.set('k', 'v')", "?k=v"),
        ("http://ex.com/", "u.searchParams.append('k', 'v')", "?k=v"),
        ("http://ex.com/?k=v", "u.searchParams.delete('k')", ""),
        (
            "http://ex.com/?b=2&a=1",
            "u.searchParams.sort()",
            "?a=1&b=2",
        ),
        (
            "http://ex.com/",
            "u.searchParams.set('a', '1'); u.searchParams.set('b', '2')",
            "?a=1&b=2",
        ),
    ] {
        let src = format!("const u = new URL('{start_url}'); {mutation}; u.search");
        assert_eq!(text(eval(&rt, &src)), want, "{mutation}");
    }
}

#[test]
fn searchparams_mutation_propagates_to_href() {
    let rt = runtime();
    assert_eq!(
        text(eval(
            &rt,
            "const u = new URL('http://ex.com/path'); u.searchParams.set('k', 'v'); u.href",
        )),
        "http://ex.com/path?k=v",
    );
}

#[test]
fn urlsearchparams_accepts_iterable_init() {
    let rt = runtime();
    for (setup, init, want) in [
        (
            "const fd = new FormData(); fd.append('a', '1'); fd.append('b', '2')",
            "new URLSearchParams(fd)",
            "a=1&b=2",
        ),
        (
            "const fd = new FormData(); fd.append('x', 'y')",
            "new URLSearchParams(fd)",
            "x=y",
        ),
        (
            "",
            "new URLSearchParams(new URLSearchParams('a=1&b=2'))",
            "a=1&b=2",
        ),
    ] {
        let src = format!("{setup}; const init = {init}; init.toString()");
        assert_eq!(text(eval(&rt, &src)), want, "{init}");
    }
}

#[test]
fn create_and_revoke_object_url() {
    let rt = runtime();
    assert!(boolean(eval(
        &rt,
        r#"
        const url = URL.createObjectURL(new Blob(['hi']));
        url.startsWith('blob:') && URL.revokeObjectURL(url) === true
    "#,
    )));
}
