import patchHxOnIndex from "./patch-happy-dom-hxon-index.js";
import SyncFetchScriptBuilder from "happy-dom/lib/fetch/utilities/SyncFetchScriptBuilder.js";

export default function patch(win) {
    // -----------------------------------------------------------------------------------
    // SyncFetchScriptBuilder.getScript — replace the "spawn node -e <script>" script
    // generation with a plain envelope object for our node:child_process polyfill's
    // execFileSync (see node-child-process.js). Both ends are our own code passing an
    // in-heap object, so no serialization is needed here (unlike the real subprocess
    // this used to emulate, which had to shuttle everything through a text pipe).
    // -----------------------------------------------------------------------------------
    SyncFetchScriptBuilder.getScript = function getScript(request) {
        return {
            __sync_fetch__: true,
            url: request.url.href,
            method: request.method,
            headers: request.headers ?? {},
            body: request.body ?? null,
        };
    };

    // -----------------------------------------------------------------------------------
    // Event.timeStamp — happy-dom's Event class sets `this[timeStamp] = performance.now()`
    // as a class field, i.e. it calls the live, user-overridable `performance.now` on every
    // event construction. Real browsers compute timeStamp via an internal engine clock that
    // application code can never observe or intercept by redefining window.performance.now.
    // This matters because code that mocks performance.now for its own purposes (e.g. an
    // htmx test measuring hx-live's recompute timing) can have its call-count bookkeeping
    // silently perturbed by unrelated event dispatches (htmx.process() alone fires several
    // lifecycle CustomEvents) that have nothing to do with what's being measured.
    // Fix: wrap CustomEvent's constructor to swap in a real, captured-at-startup clock only
    // for the synchronous duration of the (immutable) base Event field initializer, then
    // restore whatever was installed before (a test's mock, or nothing) — so from the
    // outside, this constructor never appears to have called performance.now() at all.
    // -----------------------------------------------------------------------------------
    {
        const _RealCustomEvent = win.CustomEvent;
        const _realNow = win.performance.now.bind(win.performance);
        // Assign to globalThis, not win: bare identifiers like `new CustomEvent(...)` in
        // vendored scripts (htmx.js) resolve against globalThis, which bootstrap.js only
        // ever copies win's properties onto once, at startup (see the DOMParser patch below
        // for the same lesson).
        globalThis.CustomEvent = class CustomEvent extends _RealCustomEvent {
            constructor(type, eventInitDict) {
                // `now` normally lives on deno_web's Performance.prototype, so an own
                // descriptor may not exist — restore by deleting rather than redefining.
                const desc = Object.getOwnPropertyDescriptor(win.performance, "now");
                Object.defineProperty(win.performance, "now", {
                    value: _realNow,
                    configurable: true,
                    writable: true,
                });
                try {
                    super(type, eventInitDict);
                } finally {
                    if (desc) Object.defineProperty(win.performance, "now", desc);
                    else delete win.performance.now;
                }
            }
        };
    }

    patchHxOnIndex(win);

    // -----------------------------------------------------------------------------------
    // Document.parseHTMLUnsafe — static method real browsers expose to parse an HTML
    // string into a detached Document (e.g. htmx's hx-csp extension uses it to read a
    // CSP meta tag out of response text without touching the live document).
    // happy-dom has no equivalent; DOMParser.parseFromString(html, "text/html") does the
    // same parse, just via an instance instead of a static call.
    // -----------------------------------------------------------------------------------
    // Uses the bare global DOMParser, not win.DOMParser: `win` is the original Window
    // instance, a separate object from globalThis (bootstrap.js only one-time-copies
    // properties across at startup).
    win.Document.parseHTMLUnsafe = (html) => new DOMParser().parseFromString(html, "text/html");

    // -----------------------------------------------------------------------------------
    // Response.prototype.bytes — a newer Fetch spec addition (real browsers shipped it a
    // few years back); happy-dom's Response still only has arrayBuffer()/text()/blob().
    // hx-multipart.js calls it directly (`new Response(this.body).bytes()`).
    // -----------------------------------------------------------------------------------
    if (typeof win.Response.prototype.bytes !== "function") {
        win.Response.prototype.bytes = async function () {
            return new Uint8Array(await this.arrayBuffer());
        };
    }
}
