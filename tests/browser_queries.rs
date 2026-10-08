//! `browser::Page`/`browser::Element`'s read-only query surface, `eval()`'s completion-value
//! and global-scope semantics, and `load()`'s in-place document replacement -- driven directly
//! by `tokio::test`, no pyo3/Python involved.

mod common;

use _miniclient::browser::Browser;
use _miniclient::runtime::Wait;
use common::decode_outcome;

#[tokio::test]
async fn load_sets_body() {
    let page = Browser::new("{}", false).new_page();
    page.load("<p id='msg'>hello</p>").await.unwrap();
    let body = page.find("body", None).await.unwrap().unwrap();
    assert_eq!(body.inner_html().await.unwrap(), r#"<p id="msg">hello</p>"#);
}

#[tokio::test]
async fn load_replaces_body_in_place() {
    let page = Browser::new("{}", false).new_page();
    page.load("<span id='a'>first</span>").await.unwrap();
    page.load("<span id='b'>second</span>").await.unwrap();
    let b = page.find("#b", None).await.unwrap().unwrap();
    assert_eq!(b.inner_html().await.unwrap(), "second");
    assert!(page.find("#a", None).await.unwrap().is_none());
}

#[tokio::test]
async fn find_returns_none_for_missing() {
    let page = Browser::new("{}", false).new_page();
    page.load("<p>hi</p>").await.unwrap();
    assert!(page.find("#does-not-exist", None).await.unwrap().is_none());
}

#[tokio::test]
async fn element_outer_html_and_text_content() {
    let page = Browser::new("{}", false).new_page();
    page.load("<div id='d'><span>inner</span> text</div>")
        .await
        .unwrap();
    let d = page.find("#d", None).await.unwrap().unwrap();
    assert_eq!(
        d.html().await.unwrap(),
        r#"<div id="d"><span>inner</span> text</div>"#
    );
    assert_eq!(d.text().await.unwrap(), "inner text");
}

#[tokio::test]
async fn element_eval_returns_completion_value_of_last_statement() {
    let page = Browser::new("{}", false).new_page();
    page.load("<div id='d'></div>").await.unwrap();
    let d = page.find("#d", None).await.unwrap().unwrap();
    let v: i64 = decode_outcome(d.eval("1; 2; 3").await);
    assert_eq!(v, 3);
}

#[tokio::test]
async fn element_eval_awaits_a_promise() {
    let page = Browser::new("{}", false).new_page();
    page.load("<div id='d'></div>").await.unwrap();
    let d = page.find("#d", None).await.unwrap().unwrap();
    let id: String = decode_outcome(d.eval("Promise.resolve(this.id)").await);
    assert_eq!(id, "d");
}

#[tokio::test]
async fn page_eval_returns_completion_value_and_awaits_a_promise() {
    let page = Browser::new("{}", false).new_page();
    let v: i64 = decode_outcome(
        page.runtime()
            .send_eval("1; 2; 3".into(), Wait::Promise)
            .await
            .unwrap(),
    );
    assert_eq!(v, 3);
    let v: i64 = decode_outcome(
        page.runtime()
            .send_eval("1; Promise.resolve(3)".into(), Wait::Promise)
            .await
            .unwrap(),
    );
    assert_eq!(v, 3);
}

#[tokio::test]
async fn page_eval_declarations_persist_on_global_scope() {
    let page = Browser::new("{}", false).new_page();
    page.runtime()
        .send_eval("var x = 1".into(), Wait::Promise)
        .await
        .unwrap()
        .unwrap();
    let x: i64 = decode_outcome(
        page.runtime()
            .send_eval("x".into(), Wait::Promise)
            .await
            .unwrap(),
    );
    assert_eq!(x, 1);
}

#[tokio::test]
async fn page_eval_allows_implicit_global_assignment() {
    let page = Browser::new("{}", false).new_page();
    page.runtime()
        .send_eval("undeclaredGlobal = 1".into(), Wait::Promise)
        .await
        .unwrap()
        .unwrap();
    let v: i64 = decode_outcome(
        page.runtime()
            .send_eval("undeclaredGlobal".into(), Wait::Promise)
            .await
            .unwrap(),
    );
    assert_eq!(v, 1);
}

/// Unlike the top-level `Page::eval()`, `Element::eval()` runs inside a strict-mode function
/// (see `browser::element_js`), so an undeclared assignment throws instead of creating a global.
#[tokio::test]
async fn element_eval_blocks_implicit_global_assignment() {
    let page = Browser::new("{}", false).new_page();
    page.load("<div id='d'></div>").await.unwrap();
    let d = page.find("#d", None).await.unwrap().unwrap();
    let err = d.eval("undeclaredGlobal = 1").await.unwrap_err();
    assert!(format!("{err:?}").contains("undeclaredGlobal"));
}

#[tokio::test]
async fn element_parent_walks_up_the_tree() {
    let page = Browser::new("{}", false).new_page();
    page.load("<div id='d'><span id='s'>hi</span></div>")
        .await
        .unwrap();
    let s = page.find("#s", None).await.unwrap().unwrap();
    let parent = s.parent().await.unwrap().expect("span has a parent");
    assert_eq!(parent.attr("id").await.unwrap(), Some("d".to_string()));
}

/// `<html>`'s parent is the document node, not an `Element`, so `parentElement` is `null`.
#[tokio::test]
async fn element_parent_is_none_for_root_html_element() {
    let page = Browser::new("{}", false).new_page();
    let html = page.find("html", None).await.unwrap().unwrap();
    assert!(html.parent().await.unwrap().is_none());
}

#[tokio::test]
async fn fill_sets_value_and_dispatches_change() {
    let page = Browser::new("{}", false).new_page();
    page.load("<input id='inp' value='old'>").await.unwrap();
    page.runtime()
        .send_eval(
            "document.getElementById('inp').addEventListener('change', \
             () => { window.__fired = true; })"
                .into(),
            Wait::Promise,
        )
        .await
        .unwrap()
        .unwrap();
    let inp = page.find("#inp", None).await.unwrap().unwrap();
    inp.fill("new").await.unwrap();
    let value: String = decode_outcome(
        page.runtime()
            .send_eval("document.querySelector('#inp').value".into(), Wait::Promise)
            .await
            .unwrap(),
    );
    assert_eq!(value, "new");
    let fired: bool = decode_outcome(
        page.runtime()
            .send_eval("window.__fired".into(), Wait::Promise)
            .await
            .unwrap(),
    );
    assert!(fired);
}

#[tokio::test]
async fn type_sets_value_and_dispatches_input() {
    let page = Browser::new("{}", false).new_page();
    page.load("<input id='inp' value='old'>").await.unwrap();
    page.runtime()
        .send_eval(
            "document.getElementById('inp').addEventListener('input', \
             () => { window.__fired = true; })"
                .into(),
            Wait::Promise,
        )
        .await
        .unwrap()
        .unwrap();
    let inp = page.find("#inp", None).await.unwrap().unwrap();
    inp.r#type("new").await.unwrap();
    let value: String = decode_outcome(
        page.runtime()
            .send_eval("document.querySelector('#inp').value".into(), Wait::Promise)
            .await
            .unwrap(),
    );
    assert_eq!(value, "new");
    let fired: bool = decode_outcome(
        page.runtime()
            .send_eval("window.__fired".into(), Wait::Promise)
            .await
            .unwrap(),
    );
    assert!(fired);
}

#[tokio::test]
async fn fill_works_on_a_textarea() {
    let page = Browser::new("{}", false).new_page();
    page.load("<textarea id='ta'>old</textarea>").await.unwrap();
    let ta = page.find("#ta", None).await.unwrap().unwrap();
    ta.fill("new").await.unwrap();
    let value: String = decode_outcome(
        page.runtime()
            .send_eval("document.querySelector('#ta').value".into(), Wait::Promise)
            .await
            .unwrap(),
    );
    assert_eq!(value, "new");
}

#[tokio::test]
async fn find_all_returns_every_match_in_order() {
    let page = Browser::new("{}", false).new_page();
    page.load("<ul><li>a</li><li>b</li><li>c</li></ul>")
        .await
        .unwrap();
    let items = page.find_all("li", None).await.unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].text().await.unwrap(), "a");
    assert_eq!(items[1].text().await.unwrap(), "b");
    assert_eq!(items[2].text().await.unwrap(), "c");
}

#[tokio::test]
async fn find_all_is_empty_for_no_matches() {
    let page = Browser::new("{}", false).new_page();
    page.load("<div>no items</div>").await.unwrap();
    assert!(page.find_all("li", None).await.unwrap().is_empty());
}

#[tokio::test]
async fn find_with_text_filters_matches() {
    let page = Browser::new("{}", false).new_page();
    page.load("<ul><li>apple</li><li>banana</li><li>apricot</li></ul>")
        .await
        .unwrap();

    // No filter: first match wins.
    let el = page.find("li", None).await.unwrap().unwrap();
    assert_eq!(el.text().await.unwrap(), "apple");

    // Substring matches multiple: first one wins.
    let el = page.find("li", Some("ap")).await.unwrap().unwrap();
    assert_eq!(el.text().await.unwrap(), "apple");

    // Substring matches a single, non-first element.
    let el = page.find("li", Some("banana")).await.unwrap().unwrap();
    assert_eq!(el.text().await.unwrap(), "banana");

    // Substring matches nothing.
    assert!(page.find("li", Some("xyz")).await.unwrap().is_none());
}

#[tokio::test]
async fn find_all_with_text_filters_every_match() {
    let page = Browser::new("{}", false).new_page();
    page.load("<ul><li>apple</li><li>banana</li><li>apricot</li></ul>")
        .await
        .unwrap();

    let all = page.find_all("li", None).await.unwrap();
    let texts: Vec<String> = collect_texts(&all).await;
    assert_eq!(texts, vec!["apple", "banana", "apricot"]);

    let some = page.find_all("li", Some("ap")).await.unwrap();
    let texts: Vec<String> = collect_texts(&some).await;
    assert_eq!(texts, vec!["apple", "apricot"]);

    assert!(page.find_all("li", Some("xyz")).await.unwrap().is_empty());
}

async fn collect_texts(els: &[_miniclient::browser::Element]) -> Vec<String> {
    let mut out = Vec::with_capacity(els.len());
    for el in els {
        out.push(el.text().await.unwrap());
    }
    out
}

#[tokio::test]
async fn element_find_returns_descendant_scoped_to_itself() {
    let page = Browser::new("{}", false).new_page();
    page.load("<div id='d'><p id='msg'>hello</p></div>")
        .await
        .unwrap();
    let d = page.find("#d", None).await.unwrap().unwrap();
    let el = d.find("#msg", None).await.unwrap().unwrap();
    assert_eq!(el.inner_html().await.unwrap(), "hello");
}

#[tokio::test]
async fn element_find_ignores_matches_outside_itself() {
    let page = Browser::new("{}", false).new_page();
    page.load("<div id='d'><p class='x'>inside</p></div><p class='x'>outside</p>")
        .await
        .unwrap();
    let d = page.find("#d", None).await.unwrap().unwrap();
    let items = d.find_all(".x", None).await.unwrap();
    assert_eq!(collect_texts(&items).await, vec!["inside"]);
}

#[tokio::test]
async fn element_find_returns_none_for_missing() {
    let page = Browser::new("{}", false).new_page();
    page.load("<div id='d'><p>hi</p></div>").await.unwrap();
    let d = page.find("#d", None).await.unwrap().unwrap();
    assert!(d.find("#does-not-exist", None).await.unwrap().is_none());
}

#[tokio::test]
async fn element_find_all_scoped_returns_elements_in_order() {
    let page = Browser::new("{}", false).new_page();
    page.load("<ul id='list'><li>a</li><li>b</li></ul>")
        .await
        .unwrap();
    let ul = page.find("#list", None).await.unwrap().unwrap();
    let items = ul.find_all("li", None).await.unwrap();
    assert_eq!(collect_texts(&items).await, vec!["a", "b"]);
}

#[tokio::test]
async fn element_find_returns_a_form_element() {
    let page = Browser::new("{}", false).new_page();
    page.load("<div id='d'><form id='f'></form></div>")
        .await
        .unwrap();
    let d = page.find("#d", None).await.unwrap().unwrap();
    let form = d.find("form", None).await.unwrap().unwrap();
    assert_eq!(form.tag_name().await.unwrap(), "FORM");
}
