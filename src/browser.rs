//! Pyo3-agnostic async core for the ergonomic DOM-query surface, mirroring the
//! `runtime::Runtime`/pyclass `Page` split in `src/lib.rs`: `browser::Page` wraps
//! `runtime::Page`, driven directly by `cargo test`/`tokio::test`, no pyo3/GIL involved.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use deno_core::serde_json;
use tokio::sync::oneshot;

use crate::ops;
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

fn json_str(s: &str) -> String {
    serde_json::to_string(s).expect("a &str always serializes to JSON")
}

fn json_opt_str(s: Option<&str>) -> String {
    s.map_or_else(|| "null".to_string(), json_str)
}

/// Wraps `code` in a strict-mode function bound to the deref'd element as `this`, throwing if
/// the element's gone (removed from the DOM, or GC'd). Binding a custom `this` requires a
/// function, so declarations inside never reach `globalThis` regardless -- unlike the
/// top-level scripts `fast_eval`/`promise_eval`/`full_eval` run directly against `globalThis` with no wrapper
/// at all. Strict mode adds one more difference from that unwrapped case: an undeclared
/// assignment throws instead of silently creating a global.
fn element_js(handle: i64, code: &str) -> String {
    format!(
        "(function () {{\n  'use strict';\n  if (!this) throw new Error('Element not found (handle {handle})');\n  return eval({});\n}}).call(__mini.deref({handle}));",
        json_str(code.trim())
    )
}

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

const PARENT_JS: &str = "
    const p = this.parentElement;
    p ? [__mini.ref(p), p.tagName] : [null, null];";

/// Wraps `code` in an IIFE so its own `const`/`let` locals don't collide with a later,
/// separate top-level eval declaring the same names in the same global lexical scope --
/// purely an implementation detail of the query JS below, unrelated to the public `eval()`
/// methods' scoping.
fn query_js(code: &str) -> String {
    format!("(() => eval({}))();", json_str(code))
}

async fn fast_eval(runtime: &runtime::Handle, source: String) -> EvalOutcome {
    runtime
        .send_eval(source, Wait::None)
        .await
        .unwrap_or_else(|_| Err(EvalError::Other("the runtime is closed".into())))
}

/// Promise-only eval -- backs the generic `eval` escape hatch, which doesn't wait for unrelated
/// pending page work.
async fn promise_eval(runtime: &runtime::Handle, source: String) -> EvalOutcome {
    runtime
        .send_eval(source, Wait::Promise)
        .await
        .unwrap_or_else(|_| Err(EvalError::Other("the runtime is closed".into())))
}

/// Settle-wait eval (promise resolved, event loop drained to idle) -- the path every mutation
/// and navigation method below uses; the generic `eval` escape hatches don't settle.
async fn full_eval(runtime: &runtime::Handle, source: String) -> EvalOutcome {
    runtime
        .send_eval(source, Wait::Settle)
        .await
        .unwrap_or_else(|_| Err(EvalError::Other("the runtime is closed".into())))
}

/// `dispatchEvent` needs the right constructor to see type-specific fields (e.g. `key` on a
/// `KeyboardEvent`); anything not listed here still works as a plain `Event`.
fn event_class(event_type: &str) -> &'static str {
    match event_type {
        "click" | "dblclick" | "mousedown" | "mouseup" | "mousemove" | "mouseover" | "mouseout"
        | "mouseenter" | "mouseleave" => "MouseEvent",
        "keydown" | "keyup" | "keypress" => "KeyboardEvent",
        "focus" | "blur" => "FocusEvent",
        "input" => "InputEvent",
        "submit" => "SubmitEvent",
        _ => "Event",
    }
}

/// Deliberately not run through `element_js`'s `this`-scoping: `__mini.awaitSettle` derefs
/// `handle` itself, so wrapping it in a second, redundant lookup would just risk the two
/// disagreeing if the element is removed between them. `js`, when given, runs against the
/// element first (used by `fill` to set the value before dispatching).
fn dispatch_js(handle: i64, event: &str, event_init_js: &str, js: &str) -> String {
    format!(
        "__mini.awaitSettle({handle}, el => {{\n  {js};\n  el.dispatchEvent(new {event_cls}({event}, {event_init_js}));\n}});",
        event_cls = event_class(event),
        event = json_str(event),
    )
}

const DEFAULT_EVENT_INIT_JS: &str = "{bubbles: true, cancelable: true}";

/// Turns a matched `(handle, tagName)` pair into an `Element`, or `None` when nothing matched --
/// the tag name travels over the wire but isn't needed here.
fn resolve_match(
    runtime: &runtime::Handle,
    outcome: EvalOutcome,
) -> Result<Option<Element>, EvalError> {
    let (handle, _tag) = decode::<MatchJs>(outcome?)?;
    Ok(handle.map(|h| Element::new(runtime.clone(), h)))
}

type MatchesJs = Vec<(i64, String)>;

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

/// Boots a blank `Runtime` on its own thread, so the caller only waits when it `recv`s. If the
/// receiver is dropped first, the thread drops the runtime it just built.
fn boot_in_background(resource_mounts_json: String) -> Receiver<runtime::Runtime> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(runtime::Runtime::new(&resource_mounts_json));
    });
    rx
}

/// Source of the runtimes its Pages run on. With `preboot`, it keeps one booted ahead in the
/// background, so taking a runtime rarely waits; without it, nothing boots in the background and
/// every take boots on the spot, saving that extra isolate.
pub struct Browser {
    resource_mounts_json: String,
    preboot: bool,
    next: Mutex<Option<Receiver<runtime::Runtime>>>,
}

impl Browser {
    pub fn new(resource_mounts_json: &str, preboot: bool) -> Arc<Self> {
        Arc::new(Self {
            resource_mounts_json: resource_mounts_json.to_string(),
            preboot,
            next: Mutex::new(preboot.then(|| boot_in_background(resource_mounts_json.to_string()))),
        })
    }

    /// Blocks until the runtime is booted.
    pub fn new_page(self: &Arc<Self>) -> Page {
        Page {
            browser: self.clone(),
            runtime: Mutex::new(Arc::new(self.take())),
            fetch_backend: Mutex::new(None),
            pristine: AtomicBool::new(true),
        }
    }

    /// Blocks until the runtime is booted, outside the lock; the next one is only spawned.
    fn take(&self) -> runtime::Runtime {
        let ready = {
            let mut next = self.next.lock().expect("buffer mutex poisoned");
            let ready = next
                .take()
                .unwrap_or_else(|| boot_in_background(self.resource_mounts_json.clone()));
            if self.preboot {
                *next = Some(boot_in_background(self.resource_mounts_json.clone()));
            }
            ready
        };
        ready.recv().expect("the runtime boot thread panicked")
    }
}

pub struct Page {
    browser: Arc<Browser>,
    // Swapped for a fresh one from `browser` on every `goto` after the first.
    runtime: Mutex<Arc<runtime::Runtime>>,
    // Installed again on each swapped-in runtime.
    fetch_backend: Mutex<Option<Arc<dyn ops::FetchBackend + Send + Sync>>>,
    // Still the blank realm `Runtime::new` booted: its first `goto` can navigate in place
    // instead of booting a second isolate. Only `goto` and `load` end it -- `eval` before the
    // first `goto` keeps it so a seeded cookie doesn't cost that saving; globals it defines
    // survive into the first document.
    pristine: AtomicBool,
}

impl Page {
    pub fn runtime(&self) -> Arc<runtime::Runtime> {
        self.runtime.lock().expect("runtime mutex poisoned").clone()
    }

    /// Return the first element matching `selector`, or `None` if nothing matches. `text`, when
    /// given, further restricts matches to elements whose `textContent` contains it.
    pub async fn find(
        &self,
        selector: &str,
        text: Option<&str>,
    ) -> Result<Option<Element>, EvalError> {
        let handle = self.runtime().handle();
        let outcome = fast_eval(&handle, query_js(&find_js("document", selector, text))).await;
        resolve_match(&handle, outcome)
    }

    /// Return every element matching `selector` (optionally filtered by `text`, see [`find`]).
    pub async fn find_all(
        &self,
        selector: &str,
        text: Option<&str>,
    ) -> Result<Vec<Element>, EvalError> {
        let handle = self.runtime().handle();
        let outcome = fast_eval(&handle, query_js(&find_all_js("document", selector, text))).await;
        resolve_matches(&handle, outcome)
    }

    /// Async because the blocking runtime method would panic inside a `tokio::test`.
    pub async fn install_fetch_backend(&self, backend: Arc<dyn ops::FetchBackend + Send + Sync>) {
        self.send_install_fetch_backend(backend)
            .await
            .expect("install channel dropped");
    }

    pub fn send_install_fetch_backend(
        &self,
        backend: Arc<dyn ops::FetchBackend + Send + Sync>,
    ) -> oneshot::Receiver<()> {
        *self.fetch_backend.lock().expect("backend mutex poisoned") = Some(backend.clone());
        self.runtime().send_install_fetch_backend(backend)
    }

    /// Fetch and load the document at `url`. The first navigation of an untouched page reuses
    /// its blank realm; every later one swaps in a blank runtime from the `Browser`, carrying
    /// cookies and history over. Elements found before then stop working.
    pub async fn goto(&self, url: &str) -> Result<(), EvalError> {
        let url = json_str(url);
        if self.pristine.swap(false, Ordering::Relaxed) {
            full_eval(
                &self.runtime().handle(),
                format!("__mini.navigateBlank({url})"),
            )
            .await?;
            return Ok(());
        }
        let from = fast_eval(&self.runtime().handle(), "__mini.goingFrom()".into()).await?;
        let from = from.expect("goingFrom() returns an object");
        // Blocks the caller until the boot ends; `preboot` is what keeps that short.
        let runtime = Arc::new(self.browser.take());
        let backend = self
            .fetch_backend
            .lock()
            .expect("backend mutex poisoned")
            .clone();
        if let Some(backend) = backend {
            runtime
                .send_install_fetch_backend(backend)
                .await
                .map_err(|_| EvalError::Other("the runtime is closed".into()))?;
        }
        let old = std::mem::replace(
            &mut *self.runtime.lock().expect("runtime mutex poisoned"),
            runtime.clone(),
        );
        // Dropping joins the isolate thread, which may wait on the GIL: never on the caller.
        std::thread::spawn(move || drop(old));
        full_eval(
            &runtime.handle(),
            format!("__mini.navigateBlank({url}, {from})"),
        )
        .await?;
        Ok(())
    }

    /// Load `html` into the document and initialize htmx, same-document (no navigation, no
    /// history change).
    pub async fn load(&self, html: &str) -> Result<(), EvalError> {
        self.pristine.store(false, Ordering::Relaxed);
        let handle = self.runtime().handle();
        full_eval(&handle, format!("__mini.documentWrite({})", json_str(html))).await?;
        Ok(())
    }
}

/// Holds a cloned handle so it remains usable after its originating [`Page`] is dropped.
pub struct Element {
    handle: i64,
    runtime: runtime::Handle,
}

impl Element {
    fn new(runtime: runtime::Handle, handle: i64) -> Self {
        Self { handle, runtime }
    }

    async fn read(&self, code: &str) -> EvalOutcome {
        fast_eval(&self.runtime, element_js(self.handle, code)).await
    }

    /// Generic escape-hatch eval, `this` bound to this element -- backs the Python facade's
    /// `Element.eval()`. Unlike [`read`](Self::read), waits for a returned promise, but not for
    /// the event loop to drain.
    pub async fn eval(&self, code: &str) -> EvalOutcome {
        promise_eval(&self.runtime, element_js(self.handle, code)).await
    }

    pub async fn text(&self) -> Result<String, EvalError> {
        decode(self.read("this.textContent").await?)
    }

    pub async fn inner_html(&self) -> Result<String, EvalError> {
        decode(self.read("this.innerHTML").await?)
    }

    pub async fn html(&self) -> Result<String, EvalError> {
        decode(self.read("this.outerHTML").await?)
    }

    pub async fn tag_name(&self) -> Result<String, EvalError> {
        decode(self.read("this.tagName").await?)
    }

    pub async fn attr(&self, name: &str) -> Result<Option<String>, EvalError> {
        decode(
            self.read(&format!("this.getAttribute({})", json_str(name)))
                .await?,
        )
    }

    /// The parent element, or `None` if it has none (e.g. the root `<html>` element, or it's
    /// been removed from the DOM).
    pub async fn parent(&self) -> Result<Option<Element>, EvalError> {
        let (handle, _tag) = decode::<MatchJs>(self.read(PARENT_JS).await?)?;
        Ok(handle.map(|h| Element::new(self.runtime.clone(), h)))
    }

    /// Like [`Page::find`], scoped to this element's descendants.
    pub async fn find(
        &self,
        selector: &str,
        text: Option<&str>,
    ) -> Result<Option<Element>, EvalError> {
        let outcome = self.read(&find_js("this", selector, text)).await;
        resolve_match(&self.runtime, outcome)
    }

    /// Like [`Page::find_all`], scoped to this element's descendants.
    pub async fn find_all(
        &self,
        selector: &str,
        text: Option<&str>,
    ) -> Result<Vec<Element>, EvalError> {
        let outcome = self.read(&find_all_js("this", selector, text)).await;
        resolve_matches(&self.runtime, outcome)
    }

    /// Dispatch a DOM event and wait for the page to settle. `event_init` defaults to
    /// `{bubbles: true, cancelable: true}` when `None`.
    pub async fn trigger(
        &self,
        event: &str,
        event_init: Option<&serde_json::Value>,
    ) -> Result<(), EvalError> {
        let event_init_js = match event_init {
            Some(init) => init.to_string(),
            None => DEFAULT_EVENT_INIT_JS.to_string(),
        };
        let js = dispatch_js(self.handle, event, &event_init_js, "");
        full_eval(&self.runtime, js).await?;
        Ok(())
    }

    /// Dispatch a click `MouseEvent` and wait for the page to settle.
    pub async fn click(&self) -> Result<(), EvalError> {
        self.trigger("click", None).await
    }

    /// Set the element's value, dispatch `change`, and wait for the page to settle.
    pub async fn fill(&self, value: &str) -> Result<(), EvalError> {
        let event_init_js = serde_json::json!({"bubbles": true}).to_string();
        let set_value_js = format!("el.value = {}", json_str(value));
        let js = dispatch_js(self.handle, "change", &event_init_js, &set_value_js);
        full_eval(&self.runtime, js).await?;
        Ok(())
    }

    /// Set the element's value, dispatch `input`, and wait for the page to settle.
    pub async fn r#type(&self, value: &str) -> Result<(), EvalError> {
        let set_value_js = format!("el.value = {}", json_str(value));
        let js = dispatch_js(self.handle, "input", DEFAULT_EVENT_INIT_JS, &set_value_js);
        full_eval(&self.runtime, js).await?;
        Ok(())
    }

    /// Submit this `<form>` element and wait for it to settle -- native navigation if
    /// unintercepted, or just a settle-wait if a script (e.g. htmx) intercepts the submit.
    /// Callable on any `Element` here: `__mini.submit` itself rejects a non-`<form>` handle, so
    /// there's no need for this core to split out its own `FormElement` to enforce that.
    pub async fn request_submit(&self) -> Result<(), EvalError> {
        full_eval(&self.runtime, format!("__mini.submit({})", self.handle)).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Browser, fast_eval, full_eval};
    use crate::runtime::Wait;

    #[tokio::test]
    async fn find_locates_an_element_and_reads_it_back() {
        let page = Browser::new("{}", false).new_page();
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
        let page = Browser::new("{}", false).new_page();
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
        let page = Browser::new("{}", false).new_page();
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
        let page = Browser::new("{}", false).new_page();
        page.runtime()
            .send_eval("document.body.innerHTML = '<p>hi</p>';".into(), Wait::None)
            .await
            .unwrap()
            .unwrap();
        let p = page.find("p", None).await.unwrap().unwrap();
        page.runtime().close();

        assert!(p.text().await.is_err());
    }

    /// `Element` clones its own runtime handle rather than borrowing from `Page` (see the
    /// struct's own doc comment), so dropping the `Page` outright -- not just closing it --
    /// must tear the element's later access down silently too.
    #[tokio::test]
    async fn element_outlives_a_dropped_page_without_panicking() {
        let page = Browser::new("{}", false).new_page();
        page.runtime()
            .send_eval("document.body.innerHTML = '<p>hi</p>';".into(), Wait::None)
            .await
            .unwrap()
            .unwrap();
        let p = page.find("p", None).await.unwrap().unwrap();
        drop(page);

        assert!(p.text().await.is_err());
    }

    /// `fast_eval` (backing read-only methods like `text`) never waits for the event loop to
    /// drain; `full_eval` (backing mutations/navigation and the public `eval()`) always does.
    /// A pending timer distinguishes them: only the settle-waiting path observes its effect.
    #[tokio::test]
    async fn fast_eval_does_not_drain_while_full_eval_does() {
        let page = Browser::new("{}", false).new_page();
        let handle = page.runtime().handle();
        page.runtime()
            .send_eval(
                "window.__ran = 0; setTimeout(() => { window.__ran = 1; }, 0);".into(),
                Wait::None,
            )
            .await
            .unwrap()
            .unwrap();

        // fast_eval: no settle-wait, so the timer never gets a chance to run.
        let before = fast_eval(&handle, "window.__ran".to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before, "0");

        // full_eval: drains the event loop to idle after evaluating its own (unrelated,
        // non-promise) script, letting the still-pending timer fire before returning.
        full_eval(&handle, "1".to_string()).await.unwrap().unwrap();
        let after = fast_eval(&handle, "window.__ran".to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after, "1");
    }
}
