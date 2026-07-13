import { Window } from "happy-dom";
import CookieStringUtility from "happy-dom/lib/cookie/urilities/CookieStringUtility.js";
import FetchCORSUtility from "happy-dom/lib/fetch/utilities/FetchCORSUtility.js";
import WindowBrowserContext from "happy-dom/lib/window/WindowBrowserContext.js";
import patchHappyDom from "./patch-happy-dom.js";

const win = new Window({
    url: globalThis.__BASE_URL__,
    settings: {
        enableJavaScriptEvaluation: true,
        fetch: { virtualServers: globalThis.__VIRTUAL_SERVERS__ ?? [] },
    },
});

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

    const res = await __host_fetch({ url, method, headers, body });

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
