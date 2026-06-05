import { Window } from 'happy-dom';

const win = new Window({ url: 'http://localhost/' });

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
globalThis.fetch = (...a) => win.fetch(...a);
globalThis.setTimeout = (...a) => win.setTimeout(...a);
globalThis.clearTimeout = (...a) => win.clearTimeout(...a);
globalThis.setInterval = (...a) => win.setInterval(...a);
globalThis.clearInterval = (...a) => win.clearInterval(...a);
