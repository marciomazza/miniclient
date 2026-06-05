import { Window } from "happy-dom";

const win = new Window({ url: globalThis.__BASE_URL__ ?? "http://localhost/" });

globalThis.window = win;
globalThis.document = win.document;
globalThis.location = win.location;
globalThis.navigator = win.navigator;
globalThis.history = win.history;
globalThis.Node = win.Node;
globalThis.Element = win.Element;
globalThis.HTMLElement = win.HTMLElement;
globalThis.Document = win.Document;
globalThis.Event = win.Event;
globalThis.CustomEvent = win.CustomEvent;
globalThis.MutationObserver = win.MutationObserver;
globalThis.AbortController = win.AbortController;
globalThis.AbortSignal = win.AbortSignal;
globalThis.Headers = win.Headers;
globalThis.Request = win.Request;
globalThis.Response = win.Response;
globalThis.FormData = win.FormData;
globalThis.URL = win.URL;
globalThis.XMLHttpRequest = win.XMLHttpRequest;
globalThis.CSSStyleSheet = win.CSSStyleSheet;
globalThis.DocumentFragment = win.DocumentFragment;
globalThis.ShadowRoot = win.ShadowRoot;
globalThis.DOMParser = win.DOMParser;
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
globalThis.setTimeout = (...a) => win.setTimeout(...a);
globalThis.clearTimeout = (...a) => win.clearTimeout(...a);
globalThis.setInterval = (...a) => win.setInterval(...a);
globalThis.clearInterval = (...a) => win.clearInterval(...a);
