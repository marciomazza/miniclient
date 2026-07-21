import { Window, PropertySymbol } from "happy-dom";
import CookieStringUtility from "happy-dom/lib/cookie/urilities/CookieStringUtility.js";
import FetchCORSUtility from "happy-dom/lib/fetch/utilities/FetchCORSUtility.js";
import WindowBrowserContext from "happy-dom/lib/window/WindowBrowserContext.js";
import patchHappyDom from "./patch-happy-dom.js";

// Snapshot the global names that already exist before Window is constructed: native
// jsrun/V8 builtins (URL, URLSearchParams, TextEncoder, ReadableStream, ...) and the
// polyfills baked into the snapshot (FormData, CSS, setTimeout/setInterval, console,
// atob/btoa, ...). Window provides its own same-named versions of several of these
// (e.g. FormData, setTimeout) that are worse fits for jsrun; the registration step
// below must not let its blanket copy overwrite something that was already there.
const _preExistingGlobals = new Set(Object.getOwnPropertyNames(globalThis));

const win = new Window({
    url: globalThis.__BASE_URL__,
    settings: {
        enableJavaScriptEvaluation: true,
        fetch: { virtualServers: globalThis.__VIRTUAL_SERVERS__ ?? [] },
    },
});

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
{
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
    win.document[PropertySymbol.defaultView] = globalThis;
}

// Copy prototype-chain Symbol-keyed methods: a couple of internal happy-dom code
// paths (dispatchError, evaluateScript) call pseudo-private Symbol methods directly
// on the bare `window` global, which is now globalThis, not the win instance they're
// defined on (Window/BrowserWindow.prototype). Own-property copying above misses
// them. Restricted to symbols only — those are invisible to normal enumeration, so
// this can't leak public API surface (addEventListener, close, ...) onto globalThis.
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

// Runs last so its patches (e.g. patch-happy-dom-url.js's globalThis.URLSearchParams
// override) are the final, authoritative values — not overwritten by the registration
// copy above, which only knows about happy-dom's unpatched classes.
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
