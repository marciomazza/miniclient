// Runs after a fresh document has been written into the page (by document.write(),
// or internally by happy-dom's own navigation machinery via browserFrame.goto()/
// frame.content=): fixes up the parsed DOM, processes htmx on it, and synthesizes
// DOMContentLoaded. happy-dom never dispatches DOMContentLoaded natively (no
// "loading" readyState, and its one native `load` event fires once for the whole
// Window's lifetime, not once per navigation). Synthesize it once this write's own
// deferred/module scripts have actually finished running, mirroring the real-browser
// rule that plain `async` scripts don't delay DOMContentLoaded but `defer`/
// `type=module` do.
globalThis.__zzz_finish_load = function () {
    __zzz_fixup_parsed_dom(document.body);
    if (typeof htmx !== "undefined") {
        htmx.process(document.body);
    }
    const pending = [...document.querySelectorAll("script[src]")]
        .filter((script) => script.type === "module" || script.defer)
        .map(
            (script) =>
                new Promise((resolve) => {
                    script.addEventListener("load", resolve, { once: true });
                    script.addEventListener("error", resolve, { once: true });
                }),
        );
    return Promise.all(pending)
        .then(() => {
            document.dispatchEvent(
                new Event("DOMContentLoaded", { bubbles: true, cancelable: false }),
            );
        })
        .then(
            () =>
                // A plain, unattributed `<script src="...">` (e.g. htmx.js) already ran by
                // the time we get here, but document.readyState is "complete", not
                // "loading", when it ran -- unlike a real browser, where such scripts
                // block the parser and always see "loading". So a script whose own
                // self-init depends on that ("if loading, wait for DOMContentLoaded, else
                // setTimeout(0)") falls into the setTimeout(0) branch here, and that timer
                // is still pending when this promise resolves. Give it one tick to fire so
                // callers of load()/goto() don't race a library's deferred self-init.
                new Promise((resolve) => setTimeout(resolve, 0)),
        );
};

// beforeContentCallback fires after goto() constructs the new Window but before it
// writes content (and runs that content's <script> tags) into it — registering here
// rather than after goto() resolves matters because this runtime has one real global object:
// a page's own scripts write to whatever globalThis is at the time they run, so
// globalThis must already mirror the new window before those scripts execute, or
// their writes (e.g. htmx.js's `globalThis.htmx = ...`) end up on the old window's
// mirror and get discarded by the next registration's reset-to-baseline sweep.
//
// Wrapped in a closure rather than passed directly: submit.js is baked into the V8
// snapshot and evaluated before bootstrap.js's module (which defines
// __zzz_register_window_globals) ever runs, so a direct reference here would freeze
// in as undefined.
const __zzz_gotoOptions = { beforeContentCallback: (win) => __zzz_register_window_globals(win) };

// Loads html as a fresh document. Used directly by Browser.load() (browser.py) and
// by the initial test-page setup (see conftest.py's HTMX_BASE_HTML). Navigates to
// about:blank first (real navigation → brand new Window, so listeners/state from
// whatever was loaded before don't carry over) rather than writing into the current
// document in place.
globalThis.__document_write = async function (html) {
    __clearAllTimers();
    const browserFrame = __zzz_get_browser_frame();
    const currentHref = location.href;
    await browserFrame.goto("about:blank", __zzz_gotoOptions);
    // Restore the pre-navigation URL: about:blank has no meaningful base for the
    // fragment's own relative URLs (hx-get="/path", action="/path", ...) to resolve
    // against.
    __zzz_set_url(browserFrame.window, browserFrame, currentHref);
    // document.write is deprecated for real browsers, but it's the only happy-dom API
    // that replaces the document and evaluates <script> tags natively.
    browserFrame.content = html;
    return __zzz_finish_load();
};

// Fetches url and loads the response body as the new document, via a real
// happy-dom navigation (fresh Window each time — no accumulating popstate listeners
// or leftover history state across goto()s). Backs Browser.goto() (browser.py).
globalThis.__zzz_fetch_and_load = async function (url) {
    __clearAllTimers();
    const browserFrame = __zzz_get_browser_frame();
    await browserFrame.goto(url, __zzz_gotoOptions);
    return __zzz_finish_load();
};

// Runs `doAction(el)`, then resolves once the page settles: happy-dom's real
// AsyncTaskManager (via window.happyDOM.waitUntilComplete(), reconnected to
// setTimeout/fetch in bootstrap.js) tracks every pending timer and in-flight fetch.
// Shared by Element.trigger()/click() (browser.py) and __zzz_submit below.
globalThis.__zzz_await_settle = async function (handle, doAction) {
    const el = __zzz_deref(handle);
    if (!el) throw new Error("Element not found (handle " + handle + ")");
    await doAction(el);
    await window.happyDOM.waitUntilComplete();
};

// happy-dom's native HTMLFormElement#submit() always sends POST bodies as
// multipart/form-data, ignoring enctype — real browsers default to
// application/x-www-form-urlencoded. Bypass it for that (common) case by
// driving BrowserFrameNavigator ourselves with a urlencoded body.
function __zzz_submit_urlencoded(el) {
    const frame = __zzz_get_browser_frame();
    __zzz_navigate({
        windowClass: document.defaultView.constructor,
        frame,
        method: "post",
        url: el.action,
        formData: new URLSearchParams(new FormData(el)),
        goToOptions: {
            referrer: frame.page.mainFrame.window.location.origin,
            // Fetch's own URLSearchParams-body handling appends ";charset=UTF-8";
            // real browsers send the bare mime type for form submissions.
            headers: { "Content-Type": "application/x-www-form-urlencoded" },
        },
    });
}

// __zzz_submit is form-only. FormElement.requestSubmit() (browser.py) calls it;
// the form's own requestSubmit() dispatches the submit event (and runs validation).
globalThis.__zzz_submit = async function (handle) {
    const browserFrame = __zzz_get_browser_frame();
    const winBefore = browserFrame.window;
    await __zzz_await_settle(handle, async (el) => {
        if (el.tagName !== "FORM") {
            throw new Error(
                "requestSubmit() only works on <form> elements (handle " + handle + ")",
            );
        }
        // Registered fresh per call, so it always runs after htmx's own submit
        // listener (added once, at htmx.process() time) — evt.defaultPrevented
        // tells us whether htmx already claimed the submission.
        el.addEventListener(
            "submit",
            (evt) => {
                if (evt.defaultPrevented) return;
                const method = (el.getAttribute("method") || "get").toLowerCase();
                const enctype = (el.enctype || "").toLowerCase();
                if (
                    method === "post" &&
                    enctype !== "multipart/form-data" &&
                    enctype !== "text/plain"
                ) {
                    evt.preventDefault();
                    __zzz_submit_urlencoded(el);
                }
            },
            { once: true },
        );
        el.requestSubmit();
    });
    // If nothing prevented the submit's default action, requestSubmit() already ran
    // happy-dom's own native HTMLFormElement submit handling, which does a real GET/POST
    // navigation through the same BrowserFrameNavigator goto() uses (see
    // HTMLFormElement.js's #submit()) — that replaces browserFrame.window with a brand
    // new Window instance, which the identity check above detects.
    if (browserFrame.window !== winBefore) {
        // __clearAllTimers() must run after __zzz_await_settle's waitUntilComplete():
        // the navigation itself relies on our custom setTimeout polyfill for its own
        // internal request-timeout guard — clearing timers earlier wipes that out too
        // and the navigation's promise never settles.
        __clearAllTimers();
        __zzz_register_window_globals(browserFrame.window);
        return __zzz_finish_load();
    }
};
