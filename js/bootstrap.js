// @ts-check
// JSDOM comes from globalThis.JSDOM instead of `import`, because that class (bundled by
// build-jsdom-bundle.mjs) is baked into the V8 snapshot: importing jsdom's module graph
// fresh on every Runtime() startup would cost the same kind of parse/compile time on every
// call. jsdom's larger bundle does cost real snapshot-restore time, not just disk space.
//
// Patches applied to this Window: XPath's ANY_TYPE default (below) and the hx-on index
// short-circuit (mini_helpers.js's installXPathShortCircuit), plus
// __mini.documentWrite/navigateFetch for navigation.

// Wrapped so page scripts, which run as global code in this same scope, can declare
// `var win`, `function sessionEntries` etc. without colliding with these names.
(function () {
    // Snapshot the global names that already exist before Window is constructed: native
    // V8 builtins (URL, URLSearchParams, TextEncoder, ReadableStream, ...) and the polyfills
    // baked into the v8_snapshot (Request, Response, CSS, setTimeout/setInterval, console,
    // atob/btoa, ...). Window provides its own same-named versions of several of these
    // (e.g. setTimeout) that are worse fits for this runtime; the registration step below
    // must not let its blanket copy overwrite something that was already there.
    //
    // `__mini` (this runtime's whole Python<->JS bridge surface: element refs, navigation,
    // settle-tracking) is already among these names by the time this line runs -- it's set up
    // by snapshot-entry.js, which is baked into the snapshot and so runs once at
    // snapshot-build time, before bootstrap.js's own fresh run on every Runtime() restore. That
    // makes it pre-existing for free: registerWindowGlobals() below only ever adds `win`'s own
    // keys onto globalThis, never recurses into an object's properties, so adding methods onto
    // the existing `__mini` object (as this file does) never needs its own entry in this set the
    // way a bare `globalThis.foo = ...` would.
    /** @type {typeof import("jsdom").JSDOM} */
    const JSDOM = /** @type {any} */ (globalThis).JSDOM;
    const _preExistingGlobals = new Set(
        /** @type {(string | symbol)[]} */ (Object.getOwnPropertyNames(globalThis)),
    );
    // Set by boot_jsruntime (src/runtime.rs) before this script runs: `null` on a first boot, or
    // `{cookies, history}` -- captured from the realm a navigation is replacing -- when carrying host
    // state forward across one. Read once, here, since cookieJar/_historyBefore/_historyAfter are
    // constructed below and never revisited.
    const _hostState = globalThis.__HOST_STATE__ ?? /** @type {any} */ ({});
    // `{entries: [{url, state}, ...], current: N}` or `null` -- destructured once, read from two
    // places below (seeding _historyBefore/_historyAfter, then applying the current entry's state).
    const _hostHistory = _hostState.history ?? null;
    const cookieJar = _hostState.cookies
        ? globalThis.__JSDOMCookieJar.deserializeSync(_hostState.cookies)
        : new globalThis.__JSDOMCookieJar();

    // Real navigation has no jsdom equivalent (location only does hash, HTMLFormElement's submit
    // is notImplementedMethod) -- a nav boots a brand new isolate instead, replaying this same
    // createDom call. Built empty: a navigation boot only has a URL, fetching and writing its HTML
    // itself once this isolate is up (see __NAVIGATION_BOOT__ below); a first boot has no content
    // at all until a caller navigates or calls Page.load().
    /** @param {string} url */
    function createDom(url) {
        const d = new JSDOM("", {
            url,
            runScripts: "dangerously",
            pretendToBeVisual: true,
            // Every http(s) request -- subresource, XHR or fetch() -- ends at mini's own op_fetch_hop
            // transport, which the Python side controls; jsdom's default would open real sockets.
            resources: { dispatcher: globalThis.__mini.baseDispatcher },
            cookieJar,
        });
        // IntersectionObserver: jsdom doesn't implement it, polyfill as a no-op.
        d.window.IntersectionObserver ??= class {
            constructor(/** @type {any} */ cb, /** @type {any} */ options) {}
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
                            dt.items
                                .filter((/** @type {any} */ i) => i.kind === "file")
                                .map((/** @type {any} */ i) => i.getAsFile()),
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
    // browser, and there's no need to keep the two objects in sync after this point. Runs once,
    // right after this isolate's one and only Window is constructed above.

    /** @param {any} win jsdom's Window, patched at runtime beyond its public typings */
    function registerWindowGlobals(win) {
        // `event` is re-installed as a live accessor below; skip the plain-value copy so it
        // isn't churned accessor->value->accessor.
        const _ignored = new Set(
            /** @type {(string | symbol)[]} */ ([
                "constructor",
                "undefined",
                "NaN",
                "global",
                "globalThis",
                "event",
            ]),
        );
        const descriptors = /** @type {any} */ (Object.getOwnPropertyDescriptors(win));
        for (const key of Reflect.ownKeys(descriptors)) {
            if (_ignored.has(key) || _preExistingGlobals.has(key)) continue;
            const winDescriptor = descriptors[key];
            if (winDescriptor.value === win) {
                win[key] = globalThis;
                winDescriptor.value = globalThis;
            }
            Object.defineProperty(globalThis, key, { ...winDescriptor, configurable: true });
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
        for (
            let proto = Object.getPrototypeOf(win);
            proto && proto !== Object.prototype;
            proto = Object.getPrototypeOf(proto)
        ) {
            for (const key of Object.getOwnPropertySymbols(proto)) {
                if (key in globalThis) continue;
                const { value } = /** @type {PropertyDescriptor} */ (
                    Object.getOwnPropertyDescriptor(proto, key)
                );
                if (typeof value === "function")
                    /** @type {any} */ (globalThis)[key] = value.bind(win);
            }
        }

        // jsdom's Window (unlike Document/Element) keeps addEventListener/removeEventListener/
        // dispatchEvent on its prototype rather than as own instance properties, so the own-
        // property copy above never picks them up -- a bare `window.addEventListener(...)` call
        // (htmx.js's popstate listener) would otherwise find nothing. Bound to `win`, like the
        // Symbol methods above: the underlying webidl2js implementation rejects any `this` that
        // isn't the real wrapper instance.
        for (const key of ["addEventListener", "removeEventListener", "dispatchEvent"]) {
            /** @type {any} */ (globalThis)[key] = /** @type {any} */ (win)[key].bind(win);
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
            win.XPathExpression.prototype.evaluate = function (
                /** @type {any} */ contextNode,
                type = 0,
                /** @type {any} */ result,
            ) {
                return origEvaluate.call(this, contextNode, type, result);
            };
        }

        globalThis.__mini.installXPathShortCircuit(win);
        globalThis.__mini.installInnerHtmlRadioFix(win);
        globalThis.__mini.installInnerText(win);

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

    // jsdom can't navigate a Window in place, so real navigation always gets a fresh one.
    // window._sessionHistory only holds that Window's own entries; the rest are parked here and
    // rejoined in sessionEntries().
    /** @type {HistoryEntries["entries"]} */
    let _historyBefore = [];
    /** @type {HistoryEntries["entries"]} */
    let _historyAfter = [];
    // Seeded from injected host state, if any; the current entry's `state` is applied to
    // `win.history` after `registerWindowGlobals(win)` below.
    if (_hostHistory) {
        const { entries, current } = _hostHistory;
        _historyBefore = entries.slice(0, current);
        _historyAfter = entries.slice(current + 1);
    }

    function sessionEntries() {
        const local = win._sessionHistory._entries.map((/** @type {any} */ e) => ({
            url: globalThis.__serializeURL(e.url),
            state: e.stateObject,
        }));
        return {
            entries: [..._historyBefore, ...local, ..._historyAfter],
            current: _historyBefore.length + win._sessionHistory._currentIndex,
        };
    }

    // Same-document rewrite, same as a real document.write() -- no new Window, no history change.
    // Used directly by Page.load() (browser.py).
    globalThis.__mini.documentWrite = async function (html) {
        win.document.open();
        win.document.write(html);
        win.document.close();
    };

    // Patched into jsdom's navigateFetch (patches/jsdom+30.1.2+001+fork.patch) -- the catch-all for
    // location.href=/link clicks that jsdom itself can't finish. Also backs Page.goto() (browser.py).
    // Just records the request; runtime.rs drives the actual isolate navigation once this eval settles.
    globalThis.__mini.navigateFetch = function (_window, url, replacement) {
        Deno.core.ops.op_request_navigation(
            new URL(url, location.href).href,
            replacement ? "replace" : "push",
        );
    };

    // Patched into jsdom's History#length getter (patches/jsdom+30.1.2+001+fork.patch): current window's
    // own entries plus the ones outside it, minus the shared current one.
    globalThis.__mini.historyLength = function () {
        return _historyBefore.length + _historyAfter.length + 1;
    };

    // Patched into jsdom's History#go() (patches/jsdom+30.1.2+001+fork.patch) for deltas past the current
    // window's own session history. Just records the request; runtime.rs drives the actual
    // navigation once this eval settles.
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

    // setTimeout/clearTimeout stay whatever pre_globals.js already installed -- settle detection
    // is the isolate's real event loop (runtime.rs), not a JS-side task counter.

    globalThis.fetch = globalThis.__mini.makeFetch(win);

    // Set by boot_jsruntime (src/runtime.rs): true only for the navigation op_request_navigation
    // drives (Page.goto()/link click/form submit), never the first boot or an externally-triggered
    // Command::Navigate (Page::navigate already brings its own document). This realm's createDom
    // above was built empty -- it must fetch its own content, since the old realm (and its
    // document) is already gone.
    // __NAVIGATION_REQUEST__ (also set by boot_jsruntime) carries the method/body/headers to reissue.
    // Fire-and-forget, not top-level await (bootstrap.js is a plain script, not a module): the
    // isolate's real event loop (runtime.rs) drains to idle after this script returns, same as
    // every other fire-and-forget fetch in this file.
    /**
     * @param {string} url
     * @param {any} request
     */
    async function fetchAndWrite(url, request) {
        let html;
        try {
            const res = await fetch(url, request);
            html = await res.text();
        } catch (err) {
            // Never revert to the old document -- it's already gone. Commit an error-state
            // document instead, marked so a caller can detect the failure.
            html = `<pre data-mini-navigation-error>${String(err)}</pre>`;
        }
        win.document.open();
        win.document.write(html);
        win.document.close();
    }

    // Page.goto() lands on a blank realm -- the page's own, if nothing navigated it yet, or a fresh one
    // swapped in -- which is what a navigation boot would build anyway, so retarget its URL and write
    // the document in place. `from` is `__mini.goingFrom()` of the realm swapped out, or absent: its
    // cookies carry over and `url` is pushed after its current history entry. Without it, `url`
    // replaces the blank history entry, like a browser's first navigation does.
    globalThis.__mini.navigateBlank = function (url, from) {
        url = new URL(url, from?.base ?? location.href).href;
        if (from) {
            globalThis.__JSDOMCookieJar.deserializeSync(from.cookies, cookieJar.store);
            _historyBefore = from.history.entries.slice(0, from.history.current + 1);
        }
        dom.reconfigure({ url });
        return fetchAndWrite(url, { method: "GET" });
    };

    // What Page.goto() carries from this realm into the one replacing it. `base` resolves the
    // relative URL the next realm is given, since that realm boots at the blank URL.
    globalThis.__mini.goingFrom = function () {
        return {
            base: location.href,
            cookies: cookieJar.serializeSync(),
            history: sessionEntries(),
        };
    };

    if (globalThis.__NAVIGATION_BOOT__) {
        fetchAndWrite(__BASE_URL__, __NAVIGATION_REQUEST__);
    }
})();
