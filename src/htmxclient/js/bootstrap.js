import { Window } from "happy-dom";
import applyURLPatches from "./urlsearch-dom-patches.js";
import applyPatchDomParser from "./patch-dom-parser.js";
import applyPatchHappyDom from "./patch-happy-dom.js";

const win = new Window({ url: globalThis.__BASE_URL__ ?? "http://localhost/" });

globalThis.window = win;
globalThis.document = win.document;
globalThis.location = win.location;
globalThis.navigator = win.navigator;
globalThis.history = win.history;

// Utility: patch a prototype method, preserving the original.
function patchMethod(proto, method, wrapper) {
    const orig = proto[method];
    proto[method] = function (...args) {
        return wrapper.call(this, orig, ...args);
    };
}

// happy-dom's pushState/replaceState don't update location.href; patch to keep them in sync.
["pushState", "replaceState"].forEach((method) => {
    const orig = win.history[method].bind(win.history);
    win.history[method] = function (state, title, url) {
        orig(state, title, url);
        if (url != null) win.location.href = String(url);
    };
});

// Bulk-assign globals from win.
const _globals = [
    "AbortController",
    "AbortSignal",
    "CSSStyleSheet",
    "CustomEvent",
    "DOMException",
    "Document",
    "DocumentFragment",
    "Element",
    "Event",
    "EventTarget",
    "FocusEvent",
    "HTMLAnchorElement",
    "HTMLButtonElement",
    "HTMLElement",
    "HTMLFormElement",
    "HTMLInputElement",
    "HTMLSelectElement",
    "HTMLTemplateElement",
    "HTMLTextAreaElement",
    "Headers",
    "InputEvent",
    "KeyboardEvent",
    "MouseEvent",
    "MutationObserver",
    "Node",
    "PointerEvent",
    "Request",
    "Response",
    "ShadowRoot",
    "SubmitEvent",
    "XMLHttpRequest",
    "customElements",
];
for (const g of _globals) globalThis[g] = win[g];
// IntersectionObserver: polyfill if absent, then expose via proxy so win and globalThis stay in sync.
win.IntersectionObserver ??= class {
    constructor(cb, options) {}
    observe() {}
    unobserve() {}
    disconnect() {}
};
Object.defineProperty(globalThis, "IntersectionObserver", {
    get() {
        return window.IntersectionObserver;
    },
    set(v) {
        window.IntersectionObserver = v;
    },
    configurable: true,
});
// Patch :disabled to account for disabled fieldset ancestor — happy-dom does not propagate
// the disabled state from <fieldset disabled> to its descendant form controls.
patchMethod(win.Element.prototype, "matches", function (_origMatches, selector) {
    const result = _origMatches.call(this, selector);
    if (!result && selector === ":disabled") {
        let p = this.parentElement;
        while (p) {
            if (p.tagName === "FIELDSET" && p.disabled) return true;
            p = p.parentElement;
        }
    }
    return result;
});
// FormData replaced by pure-JS implementation in formdata.js (loaded after this module)
applyURLPatches(win);
// Polyfill attachInternals for form-associated custom elements — happy-dom does not implement it.
// Stores the submitted value on the element as __internalsFormValue so FormData can pick it up.
patchMethod(win.HTMLElement.prototype, "attachInternals", function (_orig) {
    if (_orig) {
        try {
            return _orig.call(this);
        } catch {}
    }
    const host = this;
    return {
        setFormValue(val) {
            host.__internalsFormValue = val != null ? String(val) : null;
        },
    };
});
applyPatchDomParser(win);
const _fetchOpId = globalThis.__FETCH_OP_ID__;
globalThis.fetch = async (input, init = {}) => {
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
    const res = await __host_op_async__(_fetchOpId, { url, method, headers, body });
    return new Response(res.body != null ? new Uint8Array(res.body) : null, {
        status: res.status,
        statusText: res.statusText ?? "",
        headers: res.headers,
    });
};
// Make window.fetch a live proxy to globalThis.fetch so test mocks installed on
// globalThis (e.g. installFetchMock) are immediately visible to htmx, which reads
// window.fetch.bind(window) when building each request context.
Object.defineProperty(win, "fetch", {
    get() {
        return globalThis.fetch;
    },
    set(v) {
        globalThis.fetch = v;
    },
    configurable: true,
    enumerable: true,
});

// Timer implementation using Atomics.waitAsync — pure ECMAScript, no Python ops.
// win.setTimeout (happy-dom) never fires in this runtime because its internal timer
// queue is never drained; these replace it entirely.
{
    let _nextId = 1;
    const _active = {}; // timerId -> true
    const _intervals = {}; // intervalId -> current timerId

    const _wait = (ms) => {
        const view = new Int32Array(new SharedArrayBuffer(4));
        const r = Atomics.waitAsync(view, 0, 0, ms || 0);
        return r.async ? r.value : Promise.resolve();
    };

    globalThis.setTimeout = (fn, ms = 0, ...args) => {
        const id = _nextId++;
        _active[id] = true;
        _wait(ms).then(() => {
            if (_active[id]) {
                delete _active[id];
                fn(...args);
            }
        });
        return id;
    };

    globalThis.clearTimeout = (id) => {
        if (id != null) delete _active[id];
    };

    globalThis.setInterval = (fn, ms = 0, ...args) => {
        const intervalId = _nextId++;
        const schedule = () => {
            const timerId = _nextId++;
            _intervals[intervalId] = timerId;
            _active[timerId] = true;
            _wait(ms).then(() => {
                delete _active[timerId];
                if (intervalId in _intervals) {
                    fn(...args);
                    schedule();
                }
            });
        };
        schedule();
        return intervalId;
    };

    globalThis.clearInterval = (intervalId) => {
        const timerId = _intervals[intervalId];
        delete _intervals[intervalId];
        if (timerId != null) delete _active[timerId];
    };

    // Silently drop all pending timer callbacks — used between tests to prevent
    // stale timers from a previous test firing into the next one's context.
    globalThis.__clearAllTimers = () => {
        for (const k of Object.keys(_active)) delete _active[k];
        for (const k of Object.keys(_intervals)) delete _intervals[k];
    };
}
// happy-dom does not execute <script> elements when they are inserted into the DOM.
// Patch DOM insertion methods used by htmx during swapping so inline scripts run.
// Also patch Element.replaceWith so scripts replaced in-place during morph also run.
{
    const _runScript = (s) => {
        if (s.textContent)
            try {
                (0, eval)(s.textContent);
            } catch {}
    };
    const _evalScript = (el) => {
        if (el.nodeType !== 1) return;
        if (el.tagName === "SCRIPT") _runScript(el);
        for (const s of el.querySelectorAll("script")) _runScript(s);
    };
    const _execScripts = (nodes) => {
        for (const n of nodes) if (n) _evalScript(n);
    };
    for (const m of ["replaceChildren", "append", "before", "after", "prepend"]) {
        patchMethod(win.Element.prototype, m, function (orig, ...nodes) {
            orig.call(this, ...nodes);
            _execScripts(nodes);
        });
    }
    // insertBefore is used by htmx morph when inserting unmatched new nodes into the DOM
    patchMethod(win.Node.prototype, "insertBefore", function (_origInsertBefore, newNode, refNode) {
        const result = _origInsertBefore.call(this, newNode, refNode);
        if (this.isConnected) _execScripts([newNode]);
        return result;
    });
    // replaceWith is used during morph to swap out script nodes directly in the DOM
    patchMethod(win.Element.prototype, "replaceWith", function (_origReplaceWith, ...nodes) {
        const wasConnected = this.isConnected;
        _origReplaceWith.call(this, ...nodes);
        if (wasConnected) _execScripts(nodes);
    });
}
// happy-dom's getElementById does not respect document tree order when duplicate
// IDs exist (e.g. when htmx stores a preserved element in a pantry node appended
// after <body>).  Patch the internal Document prototype (distinct from the public
// win.Document class) to delegate to querySelectorAll, which does respect tree
// order, so the first element in document order is always returned.
{
    let _docProto = Object.getPrototypeOf(win.document);
    while (_docProto && !Object.getOwnPropertyDescriptor(_docProto, "getElementById"))
        _docProto = Object.getPrototypeOf(_docProto);
    if (_docProto) {
        patchMethod(_docProto, "getElementById", function (_origGetById, id) {
            if (!id) return _origGetById.call(this, id);
            const results = this.querySelectorAll("#" + CSS.escape(String(id)));
            return results.length > 0 ? results[0] : null;
        });
    }
}
applyPatchHappyDom(win);
// Make window behave like the global object: property writes propagate to
// globalThis so code like `window.foo = x; foo` works as in real browsers
// (where window === globalThis).  Only user-defined properties are synced —
// built-ins already on the Window prototype are left alone.
{
    const _winBuiltins = new Set();
    for (let p = win; p; p = Object.getPrototypeOf(p))
        for (const k of Object.getOwnPropertyNames(p)) _winBuiltins.add(k);
    const _winProxy = new Proxy(win, {
        set(target, prop, value) {
            target[prop] = value;
            if (typeof prop === "string" && !_winBuiltins.has(prop))
                try {
                    globalThis[prop] = value;
                } catch {}
            return true;
        },
        deleteProperty(target, prop) {
            delete target[prop];
            if (typeof prop === "string" && !_winBuiltins.has(prop))
                try {
                    delete globalThis[prop];
                } catch {}
            return true;
        },
    });
    globalThis.window = _winProxy;
}
// Set globalThis.event during event dispatch, mirroring browsers' window.event.
// Required for hx-vals="js:{...}" expressions that reference the triggering event.
// happy-dom exposes a *public* EventTarget class that differs from the internal one
// used by DOM nodes — we must patch the internal prototype found via a live element.
{
    const _probe = win.document.createElement("div");
    let _etProto = Object.getPrototypeOf(_probe);
    while (_etProto && !Object.getOwnPropertyDescriptor(_etProto, "dispatchEvent"))
        _etProto = Object.getPrototypeOf(_etProto);
    if (_etProto) {
        patchMethod(_etProto, "dispatchEvent", function (_origDispatch, evt) {
            const prev = globalThis.event;
            globalThis.event = evt;
            try {
                return _origDispatch.call(this, evt);
            } finally {
                globalThis.event = prev;
            }
        });
    }
}
