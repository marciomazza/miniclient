import { Window } from "happy-dom";

const win = new Window({ url: "http://localhost/" });

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
globalThis.setTimeout = (...a) => win.setTimeout(...a);
globalThis.clearTimeout = (...a) => win.clearTimeout(...a);
globalThis.setInterval = (...a) => win.setInterval(...a);
globalThis.clearInterval = (...a) => win.clearInterval(...a);
