globalThis.__mini ??= {};

// Navigation primitives -- element/form-agnostic. A real navigation (goto/link
// click/location.href=/form submit) goes through op_request_navigation and a fresh isolate
// (runtime.rs); __mini.navigateDocument (bootstrap.js) is the one exception, for Page.load()
// (no URL to fetch, so no isolate navigation to trigger).
(function () {
    const __mini = globalThis.__mini;

    // Loads html as a fresh document. Used directly by Page.load() (page.py) and
    // by the initial test-page setup (see conftest.py's HTMX_BASE_HTML). Keeps the current
    // URL as the new document's base, so the fragment's own relative URLs (hx-get="/path",
    // action="/path", ...) resolve the same way a real in-place document.write() would.
    __mini.documentWrite = async function (html) {
        __clearAllTimers();
        __mini.navigateDocument(html, location.href);
    };

    // Patched into jsdom's navigateFetch (see patches/jsdom+30.0.1.patch), the shared
    // dead end every real cross-document navigation hits: `location.href =` (and
    // .assign()/.replace()/component setters) and clicking a plain <a> both funnel into
    // jsdom's own navigate(), which already handles same-document fragment scrolling and
    // javascript: URLs correctly -- this only fires for the case jsdom leaves unimplemented.
    // Also backs Page.goto() (page.py). Only records the request; runtime.rs drives the
    // actual isolate navigation and self-fetch once the current top-level eval settles (see
    // op_request_navigation, src/ops.rs) -- the old isolate, and every timer on it, is
    // dropped as part of that navigation, so there's nothing to clear here.
    __mini.navigateFetch = function (_window, url) {
        Deno.core.ops.op_request_navigation(new URL(url, location.href).href, "push");
    };
})();
