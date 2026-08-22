// Browser, PropertySymbol, CookieStringUtility, FetchCORSUtility, WindowBrowserContext and
// patchHappyDom all come from globalThis.__happyDomBundle instead of `import`, because that
// bundle (built by build-happydom-bundle.mjs) is baked into the V8 snapshot: importing
// happy-dom's ~500-file module graph fresh on every Runtime() startup cost ~100ms of a
// ~110ms open_runtime() call, almost all of it V8 parsing/compiling that graph rather than
// running it.
const {
    Browser,
    PropertySymbol,
    CookieStringUtility,
    FetchCORSUtility,
    WindowBrowserContext,
    DetachedWindowAPI,
    BrowserFrameNavigator,
    patchHappyDom,
    __refreshStreamGlobals,
} = globalThis.__happyDomBundle;

// Reassert node-stream-web.js's spec-compliant ReadableStream/WritableStream/
// TransformStream onto globalThis: jsrun's own runtime installs its own (admittedly
// minimal) ReadableStream fallback once a real Runtime starts, which is after the bundle
// above was baked into the snapshot, so it can otherwise still win.
__refreshStreamGlobals();

// Snapshot the global names that already exist before Window is constructed: native
// jsrun/V8 builtins (URL, URLSearchParams, TextEncoder, ReadableStream, ...) and the
// polyfills baked into the v8_snapshot (FormData, CSS, setTimeout/setInterval, console,
// atob/btoa, ...). Window provides its own same-named versions of several of these
// (e.g. FormData, setTimeout) that are worse fits for jsrun; the registration step
// below must not let its blanket copy overwrite something that was already there.
const _preExistingGlobals = new Set(Object.getOwnPropertyNames(globalThis));

// Uses the full Browser/Page API rather than the bare `new Window()` convenience
// class: happy-dom's BrowserFrameValidator.validateFrameNavigation() explicitly
// refuses real navigation (goto()/form submission) for the main frame of a window
// built the `new Window()` way — DetachedBrowser is the only browser class with a
// `windowClass` property, and validateFrameNavigation treats that as "this is a bare
// detached Window being driven directly, not via the Browser API" and blocks it,
// falling back to a location-only, no-fetch pseudo-navigation. Browser has no
// windowClass property, so that guard never trips here, and goto() does what it says.
const browser = new Browser({
    settings: {
        enableJavaScriptEvaluation: true,
        fetch: { virtualServers: globalThis.__VIRTUAL_SERVERS__ ?? [] },
    },
});
const page = browser.newPage();
// newPage() has no way to pass an initial URL; page.mainFrame starts at about:blank.
// Set it directly (no fetch — same as what `new Window({url})` used to do) rather
// than through goto(), which would perform a real (and here pointless) navigation.
page.mainFrame.window.location[PropertySymbol.setURL](page.mainFrame, globalThis.__BASE_URL__);
const win = page.mainFrame.window;

// IntersectionObserver: happy-dom doesn't implement it, polyfill as a no-op.
win.IntersectionObserver ??= class {
    constructor(cb, options) {}
    observe() {}
    unobserve() {}
    disconnect() {}
};

// Make window behave like the global object, following the same approach as
// @happy-dom/global-registrator: copy every own property (and symbol) of the window
// instance onto globalThis, redirecting any self-reference (window.window/self/top/
// parent) to point at globalThis instead of the original instance. Afterwards
// `window === globalThis`, so code like `window.foo = x; foo` works as in a real
// browser, and there's no need to keep the two objects in sync after this point.
//
// Real navigation (browserFrame.goto()/frame.content=) replaces `win` with a brand
// new Window instance, so this registration must be re-run after every navigation —
// exposed as a global (submit.js, a plain script, has no `import` access to the
// bundle this module scope destructured above) and re-entrant:
// - First deletes every current globalThis own-key that isn't in the pre-Window
//   baseline. This wipes both the previous window's registered properties *and* any
//   arbitrary global a loaded page's own <script> set directly on globalThis (e.g.
//   `window.foo = 1`) — indirect eval means such scripts write straight to
//   globalThis, never to the underlying Window instance object, so there is nothing
//   to diff against; a full reset-to-baseline is the only way to know what's stale.
// - Then copies the new win's own properties (+ prototype-chain symbols) on top.
function registerWindowGlobals(win) {
    for (const key of Reflect.ownKeys(globalThis)) {
        if (_preExistingGlobals.has(key)) continue;
        delete globalThis[key];
    }

    // BrowserFrameNavigator's internal window class (used for every navigation past
    // the very first) doesn't auto-attach `.happyDOM` the way the top-level `Window`
    // convenience class does — reattach explicitly, and before the own-property copy
    // loop below so that loop actually picks it up onto globalThis too (setting it
    // only on `win` after the loop had already run left `globalThis.happyDOM`
    // undefined, since globalThis is a separate mirror object, not `win` itself).
    win.happyDOM = new DetachedWindowAPI(new WindowBrowserContext(win).getBrowserFrame());

    const _ignored = new Set(["constructor", "undefined", "NaN", "global", "globalThis"]);
    const keys = [
        ...Object.keys(Object.getOwnPropertyDescriptors(win)),
        ...Object.getOwnPropertySymbols(win),
    ];
    for (const key of keys) {
        if (_ignored.has(key) || _preExistingGlobals.has(key)) continue;
        const winDescriptor = Object.getOwnPropertyDescriptor(win, key);
        const globalDescriptor = Object.getOwnPropertyDescriptor(globalThis, key);
        if (globalDescriptor?.value !== undefined && globalDescriptor.value === winDescriptor.value)
            continue;
        if (winDescriptor.value === win) {
            win[key] = globalThis;
            winDescriptor.value = globalThis;
        }
        Object.defineProperty(globalThis, key, { ...winDescriptor, configurable: true });
    }
    // document.defaultView is left at happy-dom's own default (win, set by BrowserWindow's
    // constructor) rather than redirected to globalThis: HTMLFormElement's native submit
    // handling derives the window class to construct for real navigation from
    // `document.defaultView.constructor`, which only resolves correctly against the real
    // win instance.

    // Copy prototype-chain Symbol-keyed methods: a couple of internal happy-dom code
    // paths (dispatchError, evaluateScript) call pseudo-private Symbol methods directly
    // on the bare `window` global, which is now globalThis, not the win instance they're
    // defined on (Window/BrowserWindow.prototype). Own-property copying above misses
    // them. Restricted to symbols only — those are invisible to normal enumeration, so
    // this can't leak public API surface (addEventListener, close, ...) onto globalThis.
    // Re-bound every call since these close over `win` and the deletion pass above
    // just wiped the previous window's bindings.
    for (
        let proto = Object.getPrototypeOf(win);
        proto && proto !== Object.prototype;
        proto = Object.getPrototypeOf(proto)
    ) {
        for (const key of Object.getOwnPropertySymbols(proto)) {
            if (key in globalThis) continue;
            const { value } = Object.getOwnPropertyDescriptor(proto, key);
            if (typeof value === "function") globalThis[key] = value.bind(win);
        }
    }

    // Runs last, and on every navigation, not just the first: happy-dom hands each
    // window its own fresh MutationObserver/HTMLFormElement/HTMLElement/.../Response
    // classes (see WindowContextClassExtender.js — real per-window class identity,
    // not shared across instances), so prototype-level patches like the MutationObserver
    // WeakRef-GC fix only take effect on the win they're applied to. Also last so its
    // patches (e.g. patch-happy-dom-url.js's globalThis.URLSearchParams override) are
    // the final, authoritative values — not overwritten by the registration copy above,
    // which only knows about happy-dom's unpatched classes.
    patchHappyDom(win);
}
// Protected from registerWindowGlobals' own reset-to-baseline sweep below: these two
// are bootstrap-level helpers, not per-window state, and must survive every
// navigation (they're how submit.js — a plain script with no `import` of its own —
// reaches this module's WindowBrowserContext/registerWindowGlobals at all).
globalThis.__zzz_register_window_globals = registerWindowGlobals;
globalThis.__zzz_get_browser_frame = () => new WindowBrowserContext(window).getBrowserFrame();
// load()'s about:blank navigation (submit.js's __document_write) needs to restore the
// pre-navigation URL before writing content, so relative URLs in the loaded fragment
// (hx-get="/path", form action="/path", ...) still resolve against the real base URL
// instead of about:blank. Content.set/frame.goto() have no non-navigating URL-set of
// their own to call from outside this module, hence exposing the primitive here.
globalThis.__zzz_set_url = (win, browserFrame, url) => {
    win.location[PropertySymbol.setURL](browserFrame, url);
};
// Exposes happy-dom's own navigation primitive: submit.js uses it to bypass
// HTMLFormElement's native #submit(), which always sends POST bodies as
// multipart/form-data regardless of the form's actual enctype.
globalThis.__zzz_navigate = (options) => BrowserFrameNavigator.navigate(options);
_preExistingGlobals.add("__zzz_register_window_globals");
_preExistingGlobals.add("__zzz_get_browser_frame");
_preExistingGlobals.add("__zzz_set_url");
_preExistingGlobals.add("__zzz_navigate");

registerWindowGlobals(win);

// Reconnects the real happy-dom AsyncTaskManager to setTimeout and fetch, so
// window.happyDOM.waitUntilComplete() (used by submit.js's __zzz_await_settle) tracks
// every pending timer and in-flight fetch. Resolved fresh on every call, same pattern as
// the fetch override's cookie-jar lookup below, so it stays correct across navigations.
//
// setInterval/clearInterval are deliberately NOT wrapped: happy-dom's own native
// Window.setInterval calls startTimer() once at creation with no matching endTimer() until
// clearInterval() is called, so tracking polling intervals the same way (e.g. a
// hx-trigger="every 2s" element) would make waitUntilComplete() hang for the interval's
// lifetime.
//
// Returns null (rather than throwing) when there's no browserFrame yet -- same case the
// fetch override's cookie-jar lookup below already treats as expected: e.g. this fires
// mid-navigation, for goto()'s own internal request-timeout guard, before the new
// window/browserFrame pairing is fully wired up.
function __zzz_atm() {
    return new WindowBrowserContext(window).getBrowserFrame()?.[PropertySymbol.asyncTaskManager];
}
const _zzzRawSetTimeout = globalThis.setTimeout;
const _zzzRawClearTimeout = globalThis.clearTimeout;
globalThis.setTimeout = (fn, ms = 0, ...args) => {
    const id = _zzzRawSetTimeout(
        (...cbArgs) => {
            // endTimer() before the callback, not after: mirrors happy-dom's own
            // BrowserWindow.setTimeout ordering (the callback might throw).
            __zzz_atm()?.endTimer(id);
            fn(...cbArgs);
        },
        ms,
        ...args,
    );
    __zzz_atm()?.startTimer(id);
    return id;
};
globalThis.clearTimeout = (id) => {
    if (id != null) __zzz_atm()?.endTimer(id);
    _zzzRawClearTimeout(id);
};

globalThis.fetch = async (input, init = {}) => {
    const signal = input instanceof Request ? input.signal : init.signal;
    if (signal?.aborted) throw new DOMException("The operation was aborted.", "AbortError");

    let url, method, headers, body;
    if (!(input instanceof Request) && init.body instanceof FormData) {
        // happy-dom's Request constructor doesn't recognise our custom FormData and
        // would serialise it as "[object Object]" with content-type text/plain.
        // Serialize manually as multipart/form-data so the wire format matches browsers.
        const boundary = "----HxClientBoundary" + Math.random().toString(36).slice(2, 18);
        const enc = new TextEncoder();
        const chunks = [];
        for (const [name, value] of init.body) {
            chunks.push(
                enc.encode(
                    `--${boundary}\r\nContent-Disposition: form-data; name="${name}"\r\n\r\n${value}\r\n`,
                ),
            );
        }
        chunks.push(enc.encode(`--${boundary}--`));
        const total = chunks.reduce((s, c) => s + c.length, 0);
        body = new Uint8Array(total);
        let off = 0;
        for (const c of chunks) {
            body.set(c, off);
            off += c.length;
        }
        url = new URL(typeof input === "string" ? input : String(input), location.href).href;
        method = (init.method ?? "GET").toUpperCase();
        headers = {
            ...init.headers,
            "content-type": `multipart/form-data; boundary=${boundary}`,
        };
    } else {
        const req = input instanceof Request ? input : new Request(input, init);
        body = req.body ? new Uint8Array(await req.arrayBuffer()) : null;
        url = req.url;
        method = req.method;
        headers = Object.fromEntries(req.headers.entries());
    }

    // Attach the outgoing Cookie header from happy-dom's own cookie jar and store any
    // Set-Cookie response headers back into it, mirroring what happy-dom's native Fetch
    // does internally  -- otherwise this hand-rolled fetch() never touches that jar at all.
    const credentials =
        (input instanceof Request ? input.credentials : init.credentials) ?? "same-origin";
    const targetURL = new URL(url);
    const browserFrame = new WindowBrowserContext(window).getBrowserFrame();
    const hasCookieHeader = Object.keys(headers).some((k) => k.toLowerCase() === "cookie");
    if (browserFrame && !hasCookieHeader) {
        const isCORS = FetchCORSUtility.isCORS(new URL(location.href), targetURL);
        if (credentials === "include" || (credentials === "same-origin" && !isCORS)) {
            // false => include HttpOnly cookies (only document.cookie hides those, not the wire header)
            const cookies = browserFrame.page.context.cookieContainer.getCookies(targetURL, false);
            if (cookies.length > 0) headers.cookie = CookieStringUtility.cookiesToString(cookies);
        }
    }

    const atm = browserFrame?.[PropertySymbol.asyncTaskManager];
    const _taskID = atm?.startTask();
    const requestId = crypto.randomUUID();
    const onAbort = () => Deno.core.ops.op_fetch_abort(requestId);
    signal?.addEventListener("abort", onAbort);
    let res;
    try {
        res = await Deno.core.ops.op_fetch({ url, method, headers, body, id: requestId });
    } catch (err) {
        // Cancelling the Python-side task surfaces as a generic failure here --
        // reinterpret it as the spec-mandated AbortError when it was our abort.
        if (signal?.aborted) throw new DOMException("The operation was aborted.", "AbortError");
        throw err;
    } finally {
        atm?.endTask(_taskID);
        signal?.removeEventListener("abort", onAbort);
    }

    // Set-Cookie is a forbidden response header per spec: store it in the cookie jar,
    // don't let it reach Response.headers.
    const responseHeaders = res.headers.filter(([k]) => !/^set-cookie2?$/i.test(k));
    if (browserFrame) {
        for (const [k, v] of res.headers) {
            if (!/^set-cookie2?$/i.test(k)) continue;
            const cookie = CookieStringUtility.stringToCookie(targetURL, v);
            if (cookie) browserFrame.page.context.cookieContainer.addCookies([cookie]);
        }
    }
    const response = new Response(res.body != null ? new Uint8Array(res.body) : null, {
        status: res.status,
        statusText: res.statusText ?? "",
        headers: responseHeaders,
    });
    // Response.url has no ResponseInit setter — the platform fills it in as a
    // side effect of the fetch algorithm, so we do the same here.
    Object.defineProperty(response, "url", { value: res.url, configurable: true });
    return response;
};
// Protected from registerWindowGlobals' reset-to-baseline sweep, same reasoning as
// the __zzz_* helpers above: this closure already re-resolves `window`/`location`
// dynamically on every call rather than capturing a particular win, so it's valid
// for the lifetime of the runtime — it must survive every navigation rather than
// being overwritten by the plain `win.fetch` the registration copy loop would
// otherwise install.
_preExistingGlobals.add("fetch");
