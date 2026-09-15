// JSDOM comes from globalThis.JSDOM instead of `import`, because that class (bundled by
// build-jsdom-bundle.mjs) is baked into the V8 snapshot: importing jsdom's module graph
// fresh on every Runtime() startup would cost the same kind of parse/compile time that
// happy-dom's graph used to. Measured (release build, steady-state): Runtime() construction
// from the snapshot is ~33ms with jsdom, vs. ~10ms on the same machine with happy-dom's
// snapshot -- still well inside the deserialize-a-blob budget, but jsdom's larger bundle
// (~6x happy-dom's minified size) does cost real snapshot-restore time, not just disk space.
//
// No patches applied to this Window yet beyond XPath's ANY_TYPE default and the hx-on
// index short-circuit: the MutationObserver WeakRef-GC fix, cookie-jar/CORS handling in
// the fetch override, and real navigation (__zzz_navigate/__zzz_get_browser_frame) are
// all still unported below.

// Snapshot the global names that already exist before Window is constructed: native
// V8 builtins (URL, URLSearchParams, TextEncoder, ReadableStream, ...) and the polyfills
// baked into the v8_snapshot (Request, Response, CSS, setTimeout/setInterval, console,
// atob/btoa, ...). Window provides its own same-named versions of several of these
// (e.g. setTimeout) that are worse fits for this runtime; the registration step below
// must not let its blanket copy overwrite something that was already there.
const _preExistingGlobals = new Set(Object.getOwnPropertyNames(globalThis));

// `runScripts: "dangerously"` matches happy-dom's default of executing inline/loaded
// <script> tags; `pretendToBeVisual` turns on requestAnimationFrame/getComputedStyle/etc.
// jsdom takes the initial URL directly at construction, unlike happy-dom's Browser/Page
// API, which needed a Window built first and its location patched in separately.
const dom = new JSDOM("", {
    url: globalThis.__BASE_URL__,
    runScripts: "dangerously",
    pretendToBeVisual: true,
});
const win = dom.window;

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

    // Legacy `window.event` current-event global: happy-dom's fork maintains `win.event`
    // natively for the duration of each dispatch. Bridge bare `event` references (and the
    // fork's own `window.event = event` write, which lands on globalThis here because
    // self-references are redirected above) straight through to `win.event`, so there's no
    // separate mirror to keep in sync and no dispatch-time wrapper re-entering per phase.
    Object.defineProperty(globalThis, "event", {
        get: () => win.event,
        set: (value) => {
            win.event = value;
        },
        configurable: true,
    });

    // No per-window patches applied yet beyond XPath (MutationObserver WeakRef-GC fix,
    // shared SelectorParser/CSS cache).

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

    // XPathEvaluator.prototype.createExpression — short-circuit the two XPath shapes htmx
    // actually compiles (#hxOnQuery and hx-live's bind scan, both
    // `.//*[@*[starts-with(name(), "...") or @exact]]`) against patch-jsdom-hxon-index.js's
    // grow-only Set instead of walking the real XPath engine over the whole subtree on every
    // process(). A superset match is safe here (see that file's header); this is the only
    // reader of its Set. win.XPathEvaluator is a fresh class per Window, so this must be
    // reapplied every navigation like the ANY_TYPE patch above.
    {
        const HXON_INDEX_KEY = Symbol.for("miniclient.hxOnIndex");
        function hxOnShortCircuit(expr) {
            if (!/^\.\/\/\*\[@\*\[/.test(expr) || !expr.includes("starts-with(name()")) return null;
            const prefixes = [...expr.matchAll(/starts-with\(name\(\),\s*"([^"]*)"\)/g)].map(
                (m) => m[1],
            );
            const exacts = [...expr.matchAll(/(?:^|[\s([])@([a-z][\w:-]*)/gi)].map((m) => m[1]);
            const matchName = (n) => prefixes.some((p) => n.startsWith(p)) || exacts.includes(n);
            return function evaluate(ctxWrapper) {
                const ctx = globalThis.__jsdomImplForWrapper(ctxWrapper);
                const doc = ctx.ownerDocument || ctx;
                const index = doc && doc[HXON_INDEX_KEY];
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

    // Node[Deno.customInspect] — a real browser's devtools console renders a logged DOM
    // node as a short collapsed tag, never eagerly walking its full property graph. Deno's
    // console (installed in pre_globals.js) has no such special case and falls back to its
    // generic object formatter, which recurses through every own property happy-dom puts
    // on a node (internal caches, parent/child backrefs, ...). That's expensive enough to
    // matter: htmx logs `*:error` events via console.error(..., {elt, detail}) by default,
    // and elt is the live element — each such call was costing ~150ms, enough by itself to
    // blow through a reconnect-loop test's real-time assertion window (see hx-sse.js's
    // "reconnectMaxAttempts"/"reconnectMaxDelay" tests). Set per-window, not once globally:
    // each new Window gets its own fresh Node class.
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
// Protected from registerWindowGlobals' own reset-to-baseline sweep below: this is a
// bootstrap-level helper, not per-window state, and must survive every navigation
// (it's how submit.js — a plain script with no `import` of its own — reaches this
// module's registerWindowGlobals at all).
globalThis.__zzz_register_window_globals = registerWindowGlobals;
// Real navigation (browserFrame.goto(), a non-navigating URL set, and the
// multipart-bypassing submit primitive) has no jsdom equivalent yet. submit.js calls
// these; throw instead of silently doing the wrong thing.
const _notPorted = (name) => () => {
    throw new Error(`${name}: navigation is not implemented for jsdom yet`);
};
globalThis.__zzz_get_browser_frame = _notPorted("__zzz_get_browser_frame");
globalThis.__zzz_set_url = _notPorted("__zzz_set_url");
globalThis.__zzz_navigate = _notPorted("__zzz_navigate");
_preExistingGlobals.add("__zzz_register_window_globals");
_preExistingGlobals.add("__zzz_get_browser_frame");
_preExistingGlobals.add("__zzz_set_url");
_preExistingGlobals.add("__zzz_navigate");

registerWindowGlobals(win);

// window.happyDOM.waitUntilComplete() (used by submit.js's __zzz_await_settle to track
// pending timers/fetches) has no jsdom equivalent yet. Every call site already treats a
// null return as an expected "no tracker" case, so this is a harmless no-op stub.
function __zzz_atm() {
    return null;
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
        // `Request`'s body extraction only recognises its own bundled FormData class (an
        // internal brand check), not jsdom's native one exposed as the global `FormData`
        // here -- it would serialise as "[object Object]" with content-type text/plain.
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

    // No cookie jar or CORS-aware credentials handling yet: outgoing requests carry no
    // Cookie header, and Set-Cookie responses are dropped rather than stored (below).
    const atm = __zzz_atm();
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

    // Set-Cookie is a forbidden response header per spec: strip it from Response.headers
    // even though nothing stores it in a cookie jar yet.
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
// Protected from registerWindowGlobals' reset-to-baseline sweep, same reasoning as
// the __zzz_* helpers above: this closure already re-resolves `window`/`location`
// dynamically on every call rather than capturing a particular win, so it's valid
// for the lifetime of the runtime — it must survive every navigation rather than
// being overwritten by the plain `win.fetch` the registration copy loop would
// otherwise install.
_preExistingGlobals.add("fetch");
