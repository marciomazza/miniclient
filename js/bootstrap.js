// JSDOM comes from globalThis.JSDOM instead of `import`, because that class (bundled by
// build-jsdom-bundle.mjs) is baked into the V8 snapshot: importing jsdom's module graph
// fresh on every Runtime() startup would cost the same kind of parse/compile time on every
// call. jsdom's larger bundle does cost real snapshot-restore time, not just disk space.
//
// Patches applied to this Window: XPath's ANY_TYPE default and the hx-on index
// short-circuit (below), plus real navigation (__mini.navigateDocument).
//
// TODO: port the MutationObserver WeakRef-GC fix and cookie-jar/CORS handling in the fetch override.

// Snapshot the global names that already exist before Window is constructed: native
// V8 builtins (URL, URLSearchParams, TextEncoder, ReadableStream, ...) and the polyfills
// baked into the v8_snapshot (Request, Response, CSS, setTimeout/setInterval, console,
// atob/btoa, ...). Window provides its own same-named versions of several of these
// (e.g. setTimeout) that are worse fits for this runtime; the registration step below
// must not let its blanket copy overwrite something that was already there.
//
// `__mini` (this runtime's whole Python<->JS bridge surface: element refs, navigation,
// settle-tracking) is already among these names by the time this line runs -- it's set up
// by element_registry.js/submit.js, which are baked into the snapshot and so run once at
// snapshot-build time, before bootstrap.js's own fresh run on every Runtime() restore. That
// makes it pre-existing for free: registerWindowGlobals()'s reset sweep below only ever
// touches globalThis's own keys, never recurses into an object's properties, so adding
// methods onto the existing `__mini` object (as this file does) never needs its own entry
// in this set the way a bare `globalThis.foo = ...` would.
const _preExistingGlobals = new Set(Object.getOwnPropertyNames(globalThis));
globalThis.__mini ??= {};
// Set by boot_jsruntime (src/runtime.rs) before this script runs: `null` on a first boot, or
// `{cookies, history}` -- captured from the realm a navigation is replacing -- when carrying host
// state forward across one. Read once, here, since cookieJar/_historyBefore/_historyAfter are
// constructed below and never revisited.
const _hostState = globalThis.__HOST_STATE__ ?? {};
// `{entries: [{url, state}, ...], current: N}` or `null` -- destructured once, read from two
// places below (seeding _historyBefore/_historyAfter, then applying the current entry's state).
const _hostHistory = _hostState.history ?? null;
const cookieJar = _hostState.cookies
    ? globalThis.__JSDOMCookieJar.deserializeSync(_hostState.cookies)
    : new globalThis.__JSDOMCookieJar();

// Keep the old fixture cleanup path working while callers migrate to the jsdom cookie jar.
globalThis.__zzz_get_browser_frame = () => ({
    page: {
        context: { cookieContainer: { clearCookies: () => cookieJar.removeAllCookiesSync() } },
    },
});
_preExistingGlobals.add("__zzz_get_browser_frame");

// `resources` wires every http(s) subresource fetch (<script src>, <link>, ...) through this
// interceptor instead of jsdom's default (no automatic subresource fetching at all). `data:`
// and `file:` URLs never reach it -- JSDOMDispatcher resolves those itself before any
// interceptor runs. This always answers synthetically, mount hit or not: mini must never let
// undici's real network dispatcher (the interceptor chain's base) see a request, since every
// HTTP fetch mini makes -- page-level or subresource -- has to go through the same op_fetch
// transport the Python side controls.
const resourceInterceptor = __jsdomRequestInterceptor(async (request, { element }) => {
    const mounts = globalThis.__RESOURCE_MOUNTS__;
    const prefix = Object.keys(mounts).find((p) => request.url.startsWith(p));
    if (prefix === undefined) {
        try {
            return await globalThis.fetch(request);
        } catch (error) {
            // jsdom reports stylesheet failures through the link element. Convert a
            // transport failure to that path without weakening script failures.
            if (element?.localName !== "link" || !element.relList?.contains("stylesheet"))
                throw error;
            return new Response(null, { status: 404, statusText: "Not Found" });
        }
    }

    const path = `${mounts[prefix]}/${request.url.slice(prefix.length)}`;
    try {
        const bytes = Deno.core.ops.op_fs_read(path);
        return new Response(bytes, { headers: { "content-type": contentTypeFor(path) } });
    } catch {
        return new Response(null, { status: 404, statusText: "Not Found" });
    }
});

function contentTypeFor(path) {
    if (path.endsWith(".mjs") || path.endsWith(".js")) return "text/javascript";
    if (path.endsWith(".css")) return "text/css";
    if (path.endsWith(".json")) return "application/json";
    if (path.endsWith(".html")) return "text/html";
    return "application/octet-stream";
}

// Real navigation has no jsdom equivalent (location only does hash, HTMLFormElement's submit
// is notImplementedMethod) -- a nav is a new JSDOM instance, built the same way as the first.
//
// Built empty, without html, on purpose: this runtime has one real V8 global object, so a
// page's own scripts (executed by jsdom via indirect eval) write to whatever globalThis is
// at the time they run, not to the new Window instance -- if html were parsed here, its
// scripts would run and write to the *previous* window's globalThis mirror, then get wiped
// by registerWindowGlobals()'s reset-to-baseline sweep right after. Content is written
// (triggering the real parse and script execution) only once globalThis already mirrors
// this new window -- see __mini.navigateDocument below.
function createDom(url) {
    const d = new JSDOM("", {
        url,
        runScripts: "dangerously",
        pretendToBeVisual: true,
        resources: { interceptors: [resourceInterceptor] },
        cookieJar,
    });
    // IntersectionObserver: jsdom doesn't implement it, polyfill as a no-op.
    d.window.IntersectionObserver ??= class {
        constructor(cb, options) {}
        observe() {}
        unobserve() {}
        disconnect() {}
    };
    // DataTransfer: jsdom doesn't implement it either. `createDataTransfer` (from
    // @testing-library/user-event, ported rather than reimplemented) already returns a
    // spec-shaped object with items/files/getData/setData; a constructor may return an
    // object in place of `this`, so this is the whole polyfill. `createDataTransfer` itself
    // checks `typeof window.DataTransfer === 'undefined'` to pick real-vs-stub -- it must not
    // see itself here, or `new window.DataTransfer()` recurses forever, so it's hidden for the
    // duration of that one synchronous call.
    d.window.DataTransfer ??= class DataTransfer {
        constructor() {
            const self = d.window.DataTransfer;
            d.window.DataTransfer = undefined;
            let dt;
            try {
                dt = globalThis.__createDataTransfer(d.window);
            } finally {
                d.window.DataTransfer = self;
            }
            // Overrides the ported `.files` getter (fixed at construction, never reflects
            // later items.add() calls) with one derived live from `.items`, built through
            // jsdom's own FileList factory so a plain `input.files = dt.files` assignment
            // passes jsdom's webidl2js conversion.
            Object.defineProperty(dt, "files", {
                get: () =>
                    globalThis.__createRealFileList(
                        d.window,
                        dt.items.filter((i) => i.kind === "file").map((i) => i.getAsFile()),
                    ),
                configurable: true,
            });
            return dt;
        }
    };
    return d;
}
let dom = createDom(globalThis.__BASE_URL__);
const win = dom.window;

// Make window behave like the global object: copy every own property (and symbol) of
// the window instance onto globalThis, redirecting any self-reference (window.window/
// self/top/parent) to point at globalThis instead of the original instance. Afterwards
// `window === globalThis`, so code like `window.foo = x; foo` works as in a real
// browser, and there's no need to keep the two objects in sync after this point.
//
// Real navigation (__mini.navigateDocument, below) replaces `win` with a brand new
// Window instance, so this registration must be re-run after every navigation —
// exposed via __mini (submit.js, a plain script, has no `import` access to the
// bundle this module scope destructured above) and re-entrant:
// - Deletes only the globalThis own-keys that are stale: not in the pre-Window
//   baseline and not among the new win's own keys either. This still wipes an
//   arbitrary global a loaded page's own <script> set directly on globalThis (e.g.
//   `window.foo = 1`) — indirect eval means such scripts write straight to
//   globalThis, never to the underlying Window instance object — while every key the
//   new win does provide is overwritten in place below instead of deleted first.
//   Deleting everything unconditionally (the previous approach) forced globalThis
//   into V8 dictionary mode every navigation, since a fresh define-from-scratch on
//   hundreds of properties never lets a stable hidden class form; overwriting in
//   place lets repeat navigations hit plain, monomorphic property stores.
// - Then copies the new win's own properties (+ prototype-chain symbols) on top.

// XPathEvaluator.prototype.createExpression — short-circuit the two XPath shapes htmx
// actually compiles (#hxOnQuery and hx-live's bind scan, both
// `.//*[@*[starts-with(name(), "...") or @exact]]`) against patch-jsdom-hxon-index.js's
// grow-only Set instead of walking the real XPath engine over the whole subtree on every
// process(). A superset match is safe here (see that file's header); this is the only
// reader of its Set, kept on globalThis (not per-document -- see that file's header for
// why). win.XPathEvaluator is a fresh class per Window, so this must be reapplied every
// navigation like the ANY_TYPE patch above.
function installXPathShortCircuit(win) {
    const HXON_INDEX_KEY = Symbol.for("miniclient.hxOnIndex");
    function hxOnShortCircuit(expr) {
        if (!expr.startsWith(".//*[@*[") || !expr.includes("starts-with(name()")) return null;
        const prefixes = [...expr.matchAll(/starts-with\(name\(\),\s*"([^"]*)"\)/g)].map(
            (m) => m[1],
        );
        const exacts = [...expr.matchAll(/(?:^|[\s([])@([a-z][\w:-]*)/gi)].map((m) => m[1]);
        const matchName = (n) => prefixes.some((p) => n.startsWith(p)) || exacts.includes(n);
        return function evaluate(ctxWrapper) {
            const ctx = globalThis.__jsdomImplForWrapper(ctxWrapper);
            const index = globalThis[HXON_INDEX_KEY];
            const out = [];
            if (index) {
                for (const el of index) {
                    if (!el.isConnected) {
                        index.delete(el);
                        continue;
                    }
                    if (ctx !== el && !ctx.contains(el)) continue;
                    if (el.getAttributeNames().some(matchName))
                        out.push(globalThis.__jsdomWrapperForImpl(el));
                }
            }
            let i = 0;
            return { iterateNext: () => out[i++] ?? null };
        };
    }
    const origCreateExpression = win.XPathEvaluator.prototype.createExpression;
    win.XPathEvaluator.prototype.createExpression = function (expression, resolver) {
        const shortCircuit = hxOnShortCircuit(expression);
        if (shortCircuit) return { evaluate: shortCircuit };
        return origCreateExpression.call(this, expression, resolver);
    };
}

// jsdom's parser leaves multiple checked radios in one group checked when markup is
// assigned through innerHTML. Reapply the radio-group invariant after that parse; the
// checked setter already handles the sibling exclusion.
function installInnerHtmlRadioFix(win) {
    const innerHTML = Object.getOwnPropertyDescriptor(win.Element.prototype, "innerHTML");
    const originalSet = innerHTML.set;
    Object.defineProperty(win.Element.prototype, "innerHTML", {
        get: innerHTML.get,
        set(value) {
            originalSet.call(this, value);
            const groups = new Map();
            for (const radio of this.querySelectorAll('input[type="radio"][name]')) {
                if (!radio.name || !radio.checked) continue;
                const byName = groups.get(radio.form ?? radio.getRootNode()) ?? new Map();
                const previous = byName.get(radio.name);
                if (previous) previous.checked = false;
                byName.set(radio.name, radio);
                groups.set(radio.form ?? radio.getRootNode(), byName);
            }
        },
        configurable: true,
    });
}

// innerText: jsdom has no layout engine, so it never implements this (needs rendered-box
// information). styleless-innertext (ported, not reimplemented) approximates it from the
// DOM tree + computed style instead of real layout. Its own algorithm has no hidden-element
// exclusion, so a self/ancestor `hidden` attribute or display:none/visibility:hidden check
// is added here; hidden *descendants* still leak their text, an accepted gap since real
// layout info doesn't exist to detect them either.
// DOMParser-created documents have no browsing context; jsdom rejects cross-window style
// lookup, so detached fragments use textContent instead.
function installInnerText(win) {
    function isMiniHidden(el) {
        for (let node = el; node; node = node.parentElement) {
            if (node.hidden) return true;
            const ownerWindow = node.ownerDocument?.defaultView;
            if (!ownerWindow) continue;
            const style = ownerWindow.getComputedStyle(node);
            if (style.display === "none" || style.visibility === "hidden") return true;
        }
        return false;
    }
    Object.defineProperty(win.HTMLElement.prototype, "innerText", {
        get() {
            if (isMiniHidden(this)) return "";
            const ownerWindow = this.ownerDocument?.defaultView;
            if (!ownerWindow) return this.textContent;
            return globalThis.__innerText(this, {
                getComputedStyle: (el) => ownerWindow.getComputedStyle(el),
            });
        },
        set(value) {
            this.textContent = value;
        },
        configurable: true,
    });
}

function registerWindowGlobals(win) {
    // `event` is re-installed as a live accessor below; skip the plain-value copy so it
    // isn't churned accessor->value->accessor on every navigation.
    const _ignored = new Set(["constructor", "undefined", "NaN", "global", "globalThis", "event"]);
    const keys = [
        ...Object.keys(Object.getOwnPropertyDescriptors(win)),
        ...Object.getOwnPropertySymbols(win),
    ];
    const newKeys = new Set(keys);
    for (const key of Reflect.ownKeys(globalThis)) {
        if (_preExistingGlobals.has(key) || newKeys.has(key)) continue;
        delete globalThis[key];
    }

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
        // A plain assignment on an existing, matching-shape property is a monomorphic
        // store that keeps globalThis's hidden class stable; defineProperty always
        // reconfigures the property and is only needed the first time a key shows up
        // (new navigation-only globals) or when its attributes actually differ.
        if (
            globalDescriptor &&
            "value" in globalDescriptor &&
            "value" in winDescriptor &&
            globalDescriptor.writable &&
            globalDescriptor.enumerable === winDescriptor.enumerable &&
            globalDescriptor.configurable
        ) {
            globalThis[key] = winDescriptor.value;
        } else {
            Object.defineProperty(globalThis, key, { ...winDescriptor, configurable: true });
        }
    }
    // document.defaultView is left at its own default (win) rather than redirected to
    // globalThis: HTMLFormElement's native submit handling derives the window class to
    // construct for real navigation from `document.defaultView.constructor`, which only
    // resolves correctly against the real win instance.

    // Copy prototype-chain Symbol-keyed methods: jsdom internally calls pseudo-private
    // Symbol methods directly on the bare `window` global, which is globalThis, not the
    // win instance they're defined on. Own-property copying above misses them.
    // Restricted to symbols only — those are invisible to normal enumeration, so this
    // can't leak public API surface onto globalThis.
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

    // jsdom's Window (unlike Document/Element) keeps addEventListener/removeEventListener/
    // dispatchEvent on its prototype rather than as own instance properties, so the own-
    // property copy above never picks them up -- a bare `window.addEventListener(...)` call
    // (htmx.js's popstate listener) would otherwise find nothing. Bound to `win`, like the
    // Symbol methods above: the underlying webidl2js implementation rejects any `this` that
    // isn't the real wrapper instance.
    for (const key of ["addEventListener", "removeEventListener", "dispatchEvent"]) {
        globalThis[key] = win[key].bind(win);
    }

    // Legacy `window.event` current-event global: jsdom maintains `win.event` natively for
    // the duration of each dispatch. Bridge bare `event` references straight through to
    // `win.event`, so there's no separate mirror to keep in sync and no dispatch-time
    // wrapper re-entering per phase.
    Object.defineProperty(globalThis, "event", {
        get: () => win.event,
        set: (value) => {
            win.event = value;
        },
        configurable: true,
    });

    // jsdom has no Constructable Stylesheets support (adoptedStyleSheets is undefined, not
    // even an empty array): htmx.js's request-indicator CSS injection does
    // `[...document.adoptedStyleSheets, sheet]`, which throws on a bare Window otherwise.
    win.document.adoptedStyleSheets ??= [];

    globalThis.__patchJsdom(win);

    // XPathExpression.prototype.evaluate — jsdom's own XPath engine predates the webidl2js
    // pipeline that gives optional arguments their spec default, so a call with only a
    // context node (htmx's `#hxOnQuery.evaluate(root)`, run on every process()) throws
    // NOT_SUPPORTED_ERR instead of defaulting `type` to ANY_TYPE (0) per the DOM spec.
    {
        const origEvaluate = win.XPathExpression.prototype.evaluate;
        win.XPathExpression.prototype.evaluate = function (contextNode, type = 0, result) {
            return origEvaluate.call(this, contextNode, type, result);
        };
    }

    installXPathShortCircuit(win);
    installInnerHtmlRadioFix(win);
    installInnerText(win);

    // jsdom has no dialog UI, but pages still need the stateful showModal()/close() surface.
    if (win.HTMLDialogElement) {
        win.HTMLDialogElement.prototype.showModal ??= function () {
            this.open = true;
        };
        win.HTMLDialogElement.prototype.close ??= function () {
            this.open = false;
        };
    }

    // scrollIntoView: jsdom has no layout engine, so scrolling can never do anything real.
    win.Element.prototype.scrollIntoView ??= function () {};

    // Node[Deno.customInspect] — a real browser's devtools console renders a logged DOM
    // node as a short collapsed tag, never eagerly walking its full property graph. Deno's
    // console (installed in pre_globals.js) has no such special case and falls back to its
    // generic object formatter, which recurses through every own property a node exposes
    // (internal caches, parent/child backrefs, ...) — expensive enough to matter given how
    // often htmx logs `{elt, detail}` from `*:error` handlers. Set per-window, not once
    // globally: each new Window gets its own fresh Node class.
    {
        const customInspect = Symbol.for("Deno.customInspect");
        win.Node.prototype[customInspect] = function () {
            if (this.nodeType === this.TEXT_NODE)
                return `#text ${JSON.stringify((this.data ?? "").slice(0, 40))}`;
            if (typeof this.tagName !== "string") return `[Node ${this.nodeType}]`;
            const id = this.id ? ` id="${this.id}"` : "";
            return `<${this.tagName.toLowerCase()}${id}>`;
        };
    }
}
// Not per-window state, so exposed on __mini rather than copied in by
// registerWindowGlobals' own reset-to-baseline sweep below -- must survive every
// navigation (it's how submit.js — a plain script with no `import` of its own — reaches
// this module's registerWindowGlobals at all).
globalThis.__mini.registerWindowGlobals = registerWindowGlobals;

// Real navigation (window.location only handles same-document hash changes;
// HTMLFormElement's own submission algorithm is unimplemented, see
// notImplementedMethod in jsdom's HTMLFormElement-impl.js) has no jsdom equivalent --
// the only way to get a new Window/Document pair is a brand new JSDOM instance.
// Session history entries ({url, state}) outside the current Window: jsdom's own
// window._sessionHistory only ever holds the current document's entries (a fresh Window per
// navigation), so the rest are parked here and re-joined with it in sessionEntries().
let _historyBefore = [];
let _historyAfter = [];
// Seed from the injected host state (a navigation's carried-forward history), if any -- the current
// entry's `state` is applied to `win.history` once `win` exists, right after
// `registerWindowGlobals(win)` below.
if (_hostHistory) {
    const { entries, current } = _hostHistory;
    _historyBefore = entries.slice(0, current);
    _historyAfter = entries.slice(current + 1);
}

function sessionEntries() {
    const local = win._sessionHistory._entries.map((e) => ({
        url: globalThis.__serializeURL(e.url),
        state: e.stateObject,
    }));
    return {
        entries: [..._historyBefore, ...local, ..._historyAfter],
        current: _historyBefore.length + win._sessionHistory._currentIndex,
    };
}

// `history`: "push" (new entry), "replace" (current entry, e.g. document.write) or
// { traverseTo: index } (restore that entry's state, no new entry).
globalThis.__mini.navigateDocument = function (html, url, { history = "push" } = {}) {
    const { entries, current } = sessionEntries();
    let target = current;
    let state = null;
    if (history === "push") {
        entries.length = current + 1;
        target = entries.push({ url }) - 1;
    } else if (history === "replace") {
        entries[current] = { url };
    } else {
        target = history.traverseTo;
        state = entries[target].state;
    }
    _historyBefore = entries.slice(0, target);
    _historyAfter = entries.slice(target + 1);

    const previousWindow = win;
    dom = createDom(url);
    win = dom.window;
    registerWindowGlobals(win);
    if (state !== null) win.history.replaceState(state, "");
    // After the global swap: closing first would delete `_document` from a window that
    // globalThis still mirrors. jsdom keeps a closed window's whole tree alive otherwise.
    previousWindow.close();
    // Only now, with globalThis already mirroring `win`, is it safe to parse content and
    // let its <script> tags run (see createDom's comment above). open() resets the
    // just-constructed, already-"complete" empty document back to "loading", so
    // parser-inserted defer/external scripts get the right readyState and fetch
    // synchronously mid-parse, same as ticket 19's script-loading tests rely on.
    win.document.open();
    win.document.write(html);
    win.document.close();
};

// Patched into jsdom's History#length getter (see patches/jsdom+30.0.1.patch), which adds the
// current window's own entries to this count of the ones outside it, minus the shared current one.
globalThis.__mini.historyLength = function () {
    return _historyBefore.length + _historyAfter.length + 1;
};

// Patched into jsdom's History#go() (see patches/jsdom+30.0.1.patch): the hook it calls once
// a delta runs past the current window's own (always-fresh) session history. Only records the
// request; runtime.rs drives the isolate navigation and self-fetch once this eval settles.
globalThis.__mini.navigateHistory = function (delta) {
    const { entries, current } = sessionEntries();
    const target = current + delta;
    if (target < 0 || target >= entries.length) return;
    Deno.core.ops.op_request_navigation(entries[target].url, { traverseTo: target });
};

registerWindowGlobals(win);
if (_hostHistory) {
    const state = _hostHistory.entries[_hostHistory.current]?.state ?? null;
    if (state !== null) win.history.replaceState(state, "");
}

// setTimeout/clearTimeout/__clearAllTimers stay whatever pre_globals.js already installed --
// settle detection is the isolate's real event loop (runtime.rs), not a JS-side task counter.

// Shared by fetch()'s own FormData handling below and submit.js's POST branch.
globalThis.__mini.serializeMultipart = function (formData) {
    const boundary = "----HxClientBoundary" + Math.random().toString(36).slice(2, 18);
    let body = "";
    for (const [name, value] of formData) {
        body += `--${boundary}\r\nContent-Disposition: form-data; name="${name}"\r\n\r\n${value}\r\n`;
    }
    body += `--${boundary}--`;
    return { boundary, body };
};

globalThis.fetch = async (input, init = {}) => {
    const signal = input instanceof Request ? input.signal : init.signal;
    if (signal?.aborted) throw new DOMException("The operation was aborted.", "AbortError");

    let url, method, headers, body;
    if (!(input instanceof Request) && init.body instanceof FormData) {
        // `Request`'s body extraction only recognises its own bundled FormData class (an
        // internal brand check), not jsdom's native one exposed as the global `FormData`
        // here -- it would serialise as "[object Object]" with content-type text/plain.
        // Serialize manually as multipart/form-data so the wire format matches browsers.
        const { boundary, body: multipart } = globalThis.__mini.serializeMultipart(init.body);
        body = new TextEncoder().encode(multipart);
        url = new URL(typeof input === "string" ? input : String(input), location.href).href;
        method = (init.method ?? "GET").toUpperCase();
        headers = {
            ...init.headers,
            "content-type": `multipart/form-data; boundary=${boundary}`,
        };
    } else {
        // A relative string input must be resolved against location before hitting Request's
        // constructor -- unlike the FormData branch above, undici's Request never received one.
        const req =
            input instanceof Request
                ? input
                : new Request(new URL(input, location.href).href, init);
        body = req.body ? new Uint8Array(await req.arrayBuffer()) : null;
        url = req.url;
        method = req.method;
        headers = Object.fromEntries(req.headers.entries());
    }

    const requestUrl = new URL(url);
    const sameOrigin = requestUrl.origin === location.origin;
    if (init.credentials !== "omit" && (sameOrigin || init.credentials === "include")) {
        const cookie = cookieJar.getCookieStringSync(url);
        if (cookie && !Object.keys(headers).some((key) => key.toLowerCase() === "cookie")) {
            headers.cookie = cookie;
        }
    }

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
        signal?.removeEventListener("abort", onAbort);
    }

    if (init.credentials !== "omit") {
        for (const [key, value] of res.headers) {
            if (/^set-cookie2?$/i.test(key)) cookieJar.setCookieSync(value, url);
        }
    }
    // Set-Cookie is a forbidden response header per spec.
    const responseHeaders = res.headers.filter(([k]) => !/^set-cookie2?$/i.test(k));
    // Null-body statuses (204/205/304) must construct with a `null` body, not an empty one --
    // undici's Response enforces this strictly, unlike the DOM engine this used to run on.
    const responseBody = res.body != null && res.body.length > 0 ? new Uint8Array(res.body) : null;
    const response = new Response(responseBody, {
        status: res.status,
        statusText: res.statusText ?? "",
        headers: responseHeaders,
    });
    // Response.url has no ResponseInit setter — the platform fills it in as a
    // side effect of the fetch algorithm, so we do the same here.
    Object.defineProperty(response, "url", { value: res.url, configurable: true });
    return response;
};
// Protected from registerWindowGlobals' reset-to-baseline sweep: this closure already
// re-resolves `window`/`location` dynamically on every call rather than capturing a
// particular win, so it's valid for the lifetime of the runtime — it must survive every
// navigation rather than being overwritten by the plain `win.fetch` the registration
// copy loop would otherwise install. `fetch` itself can't live on `__mini` like the rest
// of the bridge surface: it's real, page-visible browser API, not internal plumbing.
_preExistingGlobals.add("fetch");

// Set by boot_jsruntime (src/runtime.rs): true only for the navigation op_request_navigation
// drives (Page.goto()/link click/form submit), never the first boot or an externally-triggered
// Command::Navigate (Page::navigate already brings its own document). This realm's createDom
// above was built empty -- it must fetch its own content, since the old realm (and its
// document) is already gone.
// __NAVIGATION_REQUEST__ (also set by boot_jsruntime) carries the method/body/headers to reissue.
// Fire-and-forget, not top-level await (bootstrap.js is a plain script, not a module): the
// isolate's real event loop (runtime.rs) drains to idle after this script returns, same as
// every other fire-and-forget fetch in this file.
if (globalThis.__NAVIGATION_BOOT__) {
    (async () => {
        let html;
        try {
            const res = await fetch(__BASE_URL__, __NAVIGATION_REQUEST__);
            if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
            html = await res.text();
        } catch (err) {
            // Never revert to the old document -- it's already gone. Commit an error-state
            // document instead, marked so a caller can detect the failure.
            html = `<pre data-mini-navigation-error>${String(err)}</pre>`;
        }
        win.document.open();
        win.document.write(html);
        win.document.close();
    })();
}
