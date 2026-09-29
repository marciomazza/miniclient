globalThis.__mini ??= {};

// Navigation primitives -- element/form-agnostic. Everything here ends in
// __mini.navigateDocument (bootstrap.js): fetch (or use given HTML) and replace the document.
// Shared by Page.load()/goto() (page.py), jsdom's own navigate() (via the
// patches/jsdom+30.0.1.patch hook) and form submission (submit.js).
(function () {
    const __mini = globalThis.__mini;

    // __mini.navigateDocument() (bootstrap.js) returns as soon as the new JSDOM instance is
    // constructed, but parser-inserted defer/type=module scripts fetch (and, per the DOM spec,
    // run) asynchronously after that. No explicit wait needed here: every caller below runs
    // inside an `eval_async`, and runtime.rs drains the isolate's real event loop to idle after
    // that call's own promise resolves, which covers those deferred fetches too.

    // Loads html as a fresh document. Used directly by Page.load() (page.py) and
    // by the initial test-page setup (see conftest.py's HTMX_BASE_HTML). Keeps the current
    // URL as the new document's base, so the fragment's own relative URLs (hx-get="/path",
    // action="/path", ...) resolve the same way a real in-place document.write() would.
    __mini.documentWrite = async function (html) {
        __clearAllTimers();
        __mini.navigateDocument(html, location.href, { history: "replace" });
    };

    // navigateOptions forwards as-is to navigateDocument (e.g. { history }).
    __mini.fetchAndNavigate = async function (url, navigateOptions) {
        // Fragments are never sent over the wire, but stay on the document's URL.
        const target = new URL(url, location.href);
        const { hash } = target;
        target.hash = "";
        const res = await fetch(target.href);
        __mini.navigateDocument(await res.text(), res.url + hash, navigateOptions);
    };

    // Fetches url and loads the response body as the new document. Backs Page.goto()
    // (page.py).
    __mini.fetchAndLoad = async function (url) {
        __clearAllTimers();
        await __mini.fetchAndNavigate(url);
    };

    // Patched into jsdom's navigateFetch (see patches/jsdom+30.0.1.patch), the shared
    // dead end every real cross-document navigation hits: `location.href =` (and
    // .assign()/.replace()/component setters) and clicking a plain <a> both funnel into
    // jsdom's own navigate(), which already handles same-document fragment scrolling and
    // javascript: URLs correctly -- this only fires for the case jsdom leaves unimplemented.
    // Fire-and-forget from jsdom's side (mirroring its own setTimeout-deferred link click), but
    // the native event-loop drain (runtime.rs) waits for this fetch regardless of who started it.
    __mini.navigateFetch = function (_window, url) {
        __clearAllTimers();
        // Not awaited by the caller (jsdom's own navigate() isn't async either) -- catch
        // here so a network failure surfaces on the console instead of as an unhandled
        // rejection.
        __mini.fetchAndNavigate(url).catch((err) => console.error("navigation failed:", err));
    };
})();
