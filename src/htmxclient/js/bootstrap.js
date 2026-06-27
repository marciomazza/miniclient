import { Window } from "happy-dom";
import patchHappyDom from "./patch-happy-dom.js";

const win = new Window({ url: globalThis.__BASE_URL__ ?? "http://localhost/" });

globalThis.window = win;
globalThis.document = win.document;
globalThis.location = win.location;
globalThis.navigator = win.navigator;
globalThis.history = win.history;

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
// FormData is intentionally absent — replaced by a pure-JS implementation in formdata.js (loaded after this module).
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
patchHappyDom(win);
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
