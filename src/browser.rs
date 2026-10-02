//! Pyo3-agnostic async core for the ergonomic DOM-query surface, mirroring the
//! `runtime::Runtime`/pyclass `Page` split in `src/lib.rs`: `browser::Page` wraps
//! `runtime::Page`, driven directly by `cargo test`/`tokio::test`, no pyo3/GIL involved.
//!
//! Ported 1:1 from `python/miniclient/page.py`'s `_dispatch_js`/`_scoped_js`/`_el_js` and the
//! query methods on `_FindMixin`/`AsyncElementBase`: same JS text, same `__mini.ref`/`deref`
//! element registry, same escaping (`serde_json`'s string encoding stands in for `json.dumps`).
//! Only the read-only half lands here -- `find`/`find_all`/`text`/`attr`/`parent`/`inner_html`,
//! all backed by `fast_eval` (no settle-wait, since a plain read never has anything to wait on).

use deno_core::serde_json;

use crate::runtime::{self, EvalError, EvalOutcome, Wait};

/// `[__mini.ref(el), el.tagName]`, or `[null, null]` when nothing matched -- the wire shape
/// `find`/`parent`'s JS hands back.
type MatchJs = (Option<i64>, Option<String>);

/// Decodes the JSON text an eval outcome carries into `T` -- `None` (JS `null`/`undefined`)
/// decodes the same as a JSON `null` would.
fn decode<T: serde::de::DeserializeOwned>(json: Option<String>) -> Result<T, EvalError> {
    let json = json.unwrap_or_else(|| "null".to_string());
    serde_json::from_str(&json).map_err(|e| EvalError::Other(e.to_string()))
}

/// Stand-in for Python's `json.dumps(s)`: a JSON string literal safe to splice into JS source.
fn json_str(s: &str) -> String {
    serde_json::to_string(s).expect("a &str always serializes to JSON")
}

/// Stand-in for `json.dumps(text)` where `text` is the optional substring filter `find`/
/// `find_all` take.
fn json_opt_str(s: Option<&str>) -> String {
    s.map_or_else(|| "null".to_string(), json_str)
}

/// Port of `page.py`'s `_scoped_js`: wraps `code` in a strict-mode function so every eval gets
/// its own scope and an explicit `this`, with `code`'s completion value as the return value.
fn scoped_js(code: &str, this_js: &str, guard: &str) -> String {
    format!(
        "(function () {{\n  'use strict';\n  {guard};\n  return eval({});\n}}).call({this_js});",
        json_str(code.trim())
    )
}

/// Port of `AsyncPage.eval`'s wrapping: runs scoped, with `this` bound to `globalThis`.
fn page_js(code: &str) -> String {
    scoped_js(code, "globalThis", "")
}

/// Port of `AsyncElementBase._el_js`: runs scoped, with `this` bound to the deref'd element, and
/// a guard that throws if the element's gone (removed from the DOM, or GC'd).
fn element_js(handle: i64, code: &str) -> String {
    let guard = format!("if (!this) throw new Error('Element not found (handle {handle})');");
    scoped_js(code, &format!("__mini.deref({handle})"), &guard)
}

/// Port of `_FindMixin.find`'s query, parameterized over `root_js` (`"document"` for `Page`,
/// `"this"` for `Element`, exactly as `_root_js` does in Python).
fn find_js(root_js: &str, selector: &str, text: Option<&str>) -> String {
    format!(
        "const text = {text};
        const matches = Array.from({root_js}.querySelectorAll({selector}));
        const match =
          text === null ? matches[0] : matches.find(m => m.textContent.includes(text));
        match ? [__mini.ref(match), match.tagName] : [null, null];",
        text = json_opt_str(text),
        selector = json_str(selector),
    )
}

/// Port of `_FindMixin.find_all`'s query.
fn find_all_js(root_js: &str, selector: &str, text: Option<&str>) -> String {
    format!(
        "const text = {text};
        Array.from({root_js}.querySelectorAll({selector}))
          .filter(m => text === null || m.textContent.includes(text))
          .map(m => [__mini.ref(m), m.tagName]);",
        text = json_opt_str(text),
        selector = json_str(selector),
    )
}

/// Port of `AsyncElementBase.parent`'s query.
const PARENT_JS: &str = "
    const p = this.parentElement;
    p ? [__mini.ref(p), p.tagName] : [null, null];";

/// No-settle-wait eval, run against a cloned runtime handle -- see `Runtime::handle`.
async fn fast_eval(runtime: &runtime::Handle, source: String) -> EvalOutcome {
    runtime
        .send_eval(source, Wait::None)
        .await
        .unwrap_or_else(|_| Err(EvalError::Other("the runtime is closed".into())))
}

/// Turns a matched `(handle, tagName)` pair into an `Element`, or `None` when nothing matched --
/// `tagName` only exists to pick a `FormElement` subclass at the pyo3/Python layer; this core
/// just needs the handle.
fn resolve_match(
    runtime: &runtime::Handle,
    outcome: EvalOutcome,
) -> Result<Option<Element>, EvalError> {
    let (handle, _tag) = decode::<MatchJs>(outcome?)?;
    Ok(handle.map(|h| Element::new(runtime.clone(), h)))
}

/// Same wire shape as [`MatchJs`], but for `find_all`: every entry is a real match, so neither
/// field is ever absent.
type MatchesJs = Vec<(i64, String)>;

/// [`resolve_match`]'s counterpart for `find_all`'s query.
fn resolve_matches(
    runtime: &runtime::Handle,
    outcome: EvalOutcome,
) -> Result<Vec<Element>, EvalError> {
    let matches: MatchesJs = decode(outcome?)?;
    Ok(matches
        .into_iter()
        .map(|(id, _tag)| Element::new(runtime.clone(), id))
        .collect())
}

/// Async, pyo3-agnostic counterpart to `AsyncPage`'s query methods, wrapping [`runtime::Page`].
pub struct Page {
    inner: runtime::Page,
}

impl Page {
    pub fn new(url: &str, resource_mounts_json: &str) -> Self {
        Self {
            inner: runtime::Page::new(url, resource_mounts_json),
        }
    }

    pub fn runtime(&self) -> &runtime::Runtime {
        self.inner.runtime()
    }

    /// Return the first element matching `selector`, or `None` if nothing matches. `text`, when
    /// given, further restricts matches to elements whose `textContent` contains it.
    pub async fn find(
        &self,
        selector: &str,
        text: Option<&str>,
    ) -> Result<Option<Element>, EvalError> {
        let handle = self.runtime().handle();
        let outcome = fast_eval(&handle, page_js(&find_js("document", selector, text))).await;
        resolve_match(&handle, outcome)
    }

    /// Return every element matching `selector` (optionally filtered by `text`, see [`find`]).
    pub async fn find_all(
        &self,
        selector: &str,
        text: Option<&str>,
    ) -> Result<Vec<Element>, EvalError> {
        let handle = self.runtime().handle();
        let outcome = fast_eval(&handle, page_js(&find_all_js("document", selector, text))).await;
        resolve_matches(&handle, outcome)
    }
}

/// Async, pyo3-agnostic counterpart to `AsyncElementBase`'s read-only surface. Identified by an
/// opaque handle into the JS-side `__mini.ref`/`deref` registry, same as `page.py`'s `Element`.
/// Holds its own cloned [`runtime::Handle`] rather than borrowing from the `Page` that produced
/// it, so it stays independently usable regardless of that `Page`'s Rust-side lifetime -- a
/// request against a since-closed runtime just surfaces as `EvalError`, same as any other eval
/// against a closed runtime.
pub struct Element {
    handle: i64,
    runtime: runtime::Handle,
}

impl Element {
    fn new(runtime: runtime::Handle, handle: i64) -> Self {
        Self { handle, runtime }
    }

    async fn eval(&self, code: &str) -> EvalOutcome {
        fast_eval(&self.runtime, element_js(self.handle, code)).await
    }

    /// `this.textContent`.
    pub async fn text(&self) -> Result<String, EvalError> {
        decode(self.eval("this.textContent").await?)
    }

    /// `this.innerHTML`.
    pub async fn inner_html(&self) -> Result<String, EvalError> {
        decode(self.eval("this.innerHTML").await?)
    }

    /// `this.getAttribute(name)`, or `None` if absent.
    pub async fn attr(&self, name: &str) -> Result<Option<String>, EvalError> {
        decode(
            self.eval(&format!("this.getAttribute({})", json_str(name)))
                .await?,
        )
    }

    /// The parent element, or `None` if it has none (e.g. the root `<html>` element, or it's
    /// been removed from the DOM).
    pub async fn parent(&self) -> Result<Option<Element>, EvalError> {
        let (handle, _tag) = decode::<MatchJs>(self.eval(PARENT_JS).await?)?;
        Ok(handle.map(|h| Element::new(self.runtime.clone(), h)))
    }

    /// Like [`Page::find`], scoped to this element's descendants.
    pub async fn find(
        &self,
        selector: &str,
        text: Option<&str>,
    ) -> Result<Option<Element>, EvalError> {
        let outcome = self.eval(&find_js("this", selector, text)).await;
        resolve_match(&self.runtime, outcome)
    }

    /// Like [`Page::find_all`], scoped to this element's descendants.
    pub async fn find_all(
        &self,
        selector: &str,
        text: Option<&str>,
    ) -> Result<Vec<Element>, EvalError> {
        let outcome = self.eval(&find_all_js("this", selector, text)).await;
        resolve_matches(&self.runtime, outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::Page;
    use crate::runtime::Wait;

    #[tokio::test]
    async fn find_locates_an_element_and_reads_it_back() {
        let page = Page::new("http://localhost/", "{}");
        page.runtime()
            .send_eval(
                r#"document.body.innerHTML = '<p id="a">hello <b>world</b></p>';"#.into(),
                Wait::None,
            )
            .await
            .expect("channel must not drop")
            .expect("script must not error");

        let p = page.find("#a", None).await.unwrap().expect("must find #a");
        assert_eq!(p.text().await.unwrap(), "hello world");
        assert_eq!(p.inner_html().await.unwrap(), "hello <b>world</b>");
        assert_eq!(p.attr("id").await.unwrap(), Some("a".to_string()));
        assert_eq!(p.attr("missing").await.unwrap(), None);

        assert!(page.find("#nope", None).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn find_with_text_filters_matches() {
        let page = Page::new("http://localhost/", "{}");
        page.runtime()
            .send_eval(
                "document.body.innerHTML = '<p>no</p><p>yes match</p>';".into(),
                Wait::None,
            )
            .await
            .unwrap()
            .unwrap();

        let p = page.find("p", Some("match")).await.unwrap().unwrap();
        assert_eq!(p.text().await.unwrap(), "yes match");

        let all = page.find_all("p", None).await.unwrap();
        assert_eq!(all.len(), 2);
        let filtered = page.find_all("p", Some("match")).await.unwrap();
        assert_eq!(filtered.len(), 1);
    }

    #[tokio::test]
    async fn parent_and_nested_find_walk_the_tree() {
        let page = Page::new("http://localhost/", "{}");
        page.runtime()
            .send_eval(
                r#"document.body.innerHTML = '<div id="outer"><span>child</span></div>';"#.into(),
                Wait::None,
            )
            .await
            .unwrap()
            .unwrap();

        let span = page.find("span", None).await.unwrap().unwrap();
        let parent = span.parent().await.unwrap().expect("span has a parent");
        assert_eq!(parent.attr("id").await.unwrap(), Some("outer".to_string()));

        let found_from_outer = parent.find("span", None).await.unwrap();
        assert!(found_from_outer.is_some());

        let body = parent.parent().await.unwrap().unwrap();
        let html = body.parent().await.unwrap().unwrap();
        assert!(
            html.parent().await.unwrap().is_none(),
            "<html> has no parent"
        );
    }

    #[tokio::test]
    async fn element_outlives_a_closed_page_without_panicking() {
        let page = Page::new("http://localhost/", "{}");
        page.runtime()
            .send_eval("document.body.innerHTML = '<p>hi</p>';".into(), Wait::None)
            .await
            .unwrap()
            .unwrap();
        let p = page.find("p", None).await.unwrap().unwrap();
        page.runtime().close();

        assert!(p.text().await.is_err());
    }
}
