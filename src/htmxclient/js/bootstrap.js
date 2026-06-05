import { Window } from "happy-dom";

const win = new Window({ url: globalThis.__BASE_URL__ ?? "http://localhost/" });

globalThis.window = win;
globalThis.document = win.document;
globalThis.location = win.location;
globalThis.navigator = win.navigator;
globalThis.history = win.history;
// happy-dom's pushState/replaceState don't update location.href; patch to keep them in sync.
["pushState", "replaceState"].forEach((method) => {
    const orig = win.history[method].bind(win.history);
    win.history[method] = function (state, title, url) {
        orig(state, title, url);
        if (url != null) win.location.href = String(url);
    };
});
globalThis.Node = win.Node;
globalThis.Element = win.Element;
globalThis.HTMLElement = win.HTMLElement;
globalThis.HTMLTemplateElement = win.HTMLTemplateElement;
globalThis.HTMLInputElement = win.HTMLInputElement;
globalThis.HTMLTextAreaElement = win.HTMLTextAreaElement;
globalThis.HTMLSelectElement = win.HTMLSelectElement;
globalThis.HTMLButtonElement = win.HTMLButtonElement;
globalThis.HTMLFormElement = win.HTMLFormElement;
globalThis.HTMLAnchorElement = win.HTMLAnchorElement;
globalThis.Document = win.Document;
globalThis.Event = win.Event;
globalThis.CustomEvent = win.CustomEvent;
globalThis.MouseEvent = win.MouseEvent;
globalThis.MutationObserver = win.MutationObserver;
// IntersectionObserver: polyfill if absent, then expose via proxy so win and globalThis stay in sync.
win.IntersectionObserver ??= class {
    constructor(cb, options) {}
    observe() {}
    unobserve() {}
    disconnect() {}
};
Object.defineProperty(globalThis, "IntersectionObserver", {
    get() { return window.IntersectionObserver; },
    set(v) { window.IntersectionObserver = v; },
    configurable: true,
});
globalThis.AbortController = win.AbortController;
globalThis.AbortSignal = win.AbortSignal;
globalThis.DOMException = win.DOMException;
globalThis.Headers = win.Headers;
globalThis.Request = win.Request;
globalThis.Response = win.Response;
// FormData replaced by pure-JS implementation in formdata.js (loaded after this module)
globalThis.URL = win.URL;
globalThis.XMLHttpRequest = win.XMLHttpRequest;
globalThis.CSSStyleSheet = win.CSSStyleSheet;
globalThis.DocumentFragment = win.DocumentFragment;
globalThis.ShadowRoot = win.ShadowRoot;
// happy-dom treats <body>...</body> as content inside body rather than as the body element itself.
// Wrapping in <html> makes it parse correctly.
{
    const _NativeDOMParser = win.DOMParser;
    globalThis.DOMParser = class {
        parseFromString(str, type) {
            if (type === "text/html" && /^\s*<body[\s>]/i.test(str))
                str = "<html>" + str + "</html>";
            return new _NativeDOMParser().parseFromString(str, type);
        }
    };
}
globalThis.CSS = {
    escape(value) {
        value = String(value);
        if (value.length === 0) return value;
        let out = "";
        for (let i = 0; i < value.length; i++) {
            const c = value.charCodeAt(i);
            if (c === 0) {
                out += "�";
                continue;
            }
            if ((c >= 0x0001 && c <= 0x001f) || c === 0x007f) {
                out += "\\" + c.toString(16) + " ";
                continue;
            }
            if (i === 0 && c >= 0x0030 && c <= 0x0039) {
                out += "\\" + c.toString(16) + " ";
                continue;
            }
            if (i === 1 && c >= 0x0030 && c <= 0x0039 && value.charCodeAt(0) === 0x002d) {
                out += "\\" + c.toString(16) + " ";
                continue;
            }
            if (i === 0 && value.length === 1 && c === 0x002d) {
                out += "\\" + value[i];
                continue;
            }
            if (
                c >= 0x0080 ||
                c === 0x002d ||
                c === 0x005f ||
                (c >= 0x0030 && c <= 0x0039) ||
                (c >= 0x0041 && c <= 0x005a) ||
                (c >= 0x0061 && c <= 0x007a)
            ) {
                out += value[i];
                continue;
            }
            out += "\\" + value[i];
        }
        return out;
    },
};
const _fetchOpId = globalThis.__FETCH_OP_ID__;
globalThis.fetch = async (input, init = {}) => {
    const req = input instanceof Request ? input : new Request(input, init);
    const body = req.body ? new Uint8Array(await req.arrayBuffer()) : null;
    const res = await __host_op_async__(_fetchOpId, {
        url: req.url,
        method: req.method,
        headers: Object.fromEntries(req.headers.entries()),
        body,
    });
    return new Response(res.body != null ? new Uint8Array(res.body) : null, {
        status: res.status,
        statusText: res.statusText ?? "",
        headers: res.headers,
    });
};
win.fetch = globalThis.fetch;

// Timer implementation backed by asyncio.sleep — integrates with the real event loop.
// win.setTimeout (happy-dom) never fires in this runtime because its internal timer
// queue is never drained; these ops replace it entirely.
{
    const _SLEEP = globalThis.__SLEEP_OP_ID__;
    const _CLEAR = globalThis.__CLEAR_TIMER_OP_ID__;
    let _nextId = 1;
    const _active = {};    // timerId  -> true
    const _intervals = {}; // intervalId -> current timerId

    globalThis.setTimeout = (fn, ms = 0, ...args) => {
        const id = _nextId++;
        _active[id] = true;
        // floating promise — resolves when sleep completes or is cancelled
        __host_op_async__(_SLEEP, { id, ms: ms || 0 }).then(({ cancelled }) => {
            if (!cancelled && _active[id]) {
                delete _active[id];
                fn(...args);
            }
        });
        return id;
    };

    globalThis.clearTimeout = (id) => {
        if (id == null) return;
        delete _active[id];
        __host_op_async__(_CLEAR, { id });
    };

    globalThis.setInterval = (fn, ms = 0, ...args) => {
        const intervalId = _nextId++;
        const schedule = () => {
            const timerId = _nextId++;
            _intervals[intervalId] = timerId;
            _active[timerId] = true;
            __host_op_async__(_SLEEP, { id: timerId, ms: ms || 0 }).then(({ cancelled }) => {
                delete _active[timerId];
                if (!cancelled && intervalId in _intervals) {
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
        if (timerId != null) {
            delete _active[timerId];
            __host_op_async__(_CLEAR, { id: timerId });
        }
    };
}
