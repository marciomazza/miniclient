// Runs after a fresh document has been written into the page (by document.write(),
// or internally by happy-dom's own navigation machinery via browserFrame.goto()/
// frame.content=): fixes up the parsed DOM, (re-)initializes htmx on it, and
// synthesizes DOMContentLoaded. happy-dom never dispatches DOMContentLoaded natively
// (no "loading" readyState, and its one native `load` event fires once for the whole
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
    return Promise.all(pending).then(() => {
        document.dispatchEvent(new Event("DOMContentLoaded", { bubbles: true, cancelable: false }));
    });
};

// beforeContentCallback fires after goto() constructs the new Window but before it
// writes content (and runs that content's <script> tags) into it — registering here
// rather than after goto() resolves matters because jsrun has one real global object:
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

// Runs `doAction(el)`, then resolves once htmx settles the request it triggered.
// If no request was triggered, calls `onNoRequest(el)` and resolves with its result
// (default: resolve with null). Shared by Element.trigger()/click() (browser.py)
// and __zzz_submit below.
globalThis.__zzz_await_htmx = function (handle, doAction, onNoRequest) {
    return new Promise((resolve, reject) => {
        let willRequest = false;
        document.addEventListener(
            "htmx:before:request",
            () => {
                willRequest = true;
            },
            { once: true },
        );
        document.addEventListener(
            "htmx:finally:request",
            () => {
                window.happyDOM.waitUntilComplete().then(resolve).catch(reject);
            },
            { once: true },
        );
        document.addEventListener(
            "htmx:error",
            (e) => {
                reject(new Error("htmx:error — " + (e.detail?.error ?? e.detail?.ctx?.status)));
            },
            { once: true },
        );

        const el = __zzz_deref(handle);
        if (!el) {
            reject(new Error("Element not found (handle " + handle + ")"));
            return;
        }

        try {
            Promise.resolve(doAction(el)).catch(reject);
        } catch (err) {
            reject(err);
        }

        setTimeout(() => {
            if (!willRequest) {
                Promise.resolve(onNoRequest?.(el)).then(resolve).catch(reject);
            }
        }, 0);
    });
};

// __zzz_submit is form-only. FormElement.requestSubmit() (browser.py) calls it;
// the form's own requestSubmit() dispatches the submit event (and runs validation).
globalThis.__zzz_submit = async function (handle) {
    const browserFrame = __zzz_get_browser_frame();
    return __zzz_await_htmx(
        handle,
        async (el) => {
            if (el.tagName !== "FORM") {
                throw new Error(
                    "requestSubmit() only works on <form> elements (handle " + handle + ")",
                );
            }
            el.requestSubmit();
        },
        async () => {
            // htmx didn't intercept => requestSubmit()'s default (unprevented) action
            // already ran happy-dom's own native HTMLFormElement submit handling, which
            // does a real GET/POST navigation through the same BrowserFrameNavigator
            // goto() uses (see HTMLFormElement.js's #submit()). Just wait for it, then
            // re-sync our globals/htmx state onto the resulting document.
            //
            // __clearAllTimers() must wait until after waitUntilComplete(): the
            // navigation itself is mid-flight at this point and relies on our custom
            // setTimeout polyfill for its own internal request-timeout guard — clearing
            // timers here wipes that out too and the navigation's promise never settles.
            await browserFrame.waitUntilComplete();
            __clearAllTimers();
            __zzz_register_window_globals(browserFrame.window);
            return __zzz_finish_load();
        },
    );
};
