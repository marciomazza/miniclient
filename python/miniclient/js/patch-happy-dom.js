import patchDomParser from "./patch-happy-dom-parser.js";
import patchHxOnIndex from "./patch-happy-dom-hxon-index.js";
import SyncFetchScriptBuilder from "happy-dom/lib/fetch/utilities/SyncFetchScriptBuilder.js";
import SelectorParser from "happy-dom/lib/query-selector/SelectorParser.js";
import CSSStyleSheet from "happy-dom/lib/css/CSSStyleSheet.js";
import CSSParser from "happy-dom/lib/css/utilities/CSSRuleParser.js";
import * as PropertySymbol from "happy-dom/lib/PropertySymbol.js";

// Parsed SelectorItem groups depend only on the selector string, but happy-dom keys its cache
// on window[querySelectorCache], which navigation discards along with the Window — so every
// goto / form submit / hx-boost re-parses every selector, and CSSParser.validateSelectorText
// re-runs the full parser on every CSS rule of every stylesheet on top. Share one cache across
// all windows instead.
{
    const CACHE = new Map();
    const origUncached = SelectorParser.prototype.getSelectorGroupsUncached;
    SelectorParser.prototype.getSelectorGroups = function (selector) {
        selector = selector.trim();
        let groups = CACHE.get(selector);
        if (groups) return groups;
        groups = origUncached.call(this, selector);
        if (CACHE.size > 5000) CACHE.clear();
        CACHE.set(selector, groups);
        return groups;
    };
}

// Parsed CSSRule trees depend only on the stylesheet text, but each CSSRule bakes in the
// window/stylesheet/parser that created it (PropertySymbol.window/parentStyleSheet/cssParser),
// and replaceSync's own text-equality check only dedupes repeat calls on the *same* instance --
// never true across navigations, since every navigation gets a fresh Window and stylesheet.
// Cache by CSS text globally and rebind the stale window/stylesheet/parser refs on every cache
// hit, so a later insertRule/appendRule or `.parentStyleSheet` read sees the current navigation.
{
    const CACHE = new Map();
    function rebind(rules, window, styleSheet) {
        for (const rule of rules) {
            rule[PropertySymbol.window] = window;
            // ponytail: two identical <link> stylesheets alive at once in the same window will
            // share these rule objects and fight over parentStyleSheet/cssParser (last one to
            // read from cache wins); harmless in practice since the content is identical and
            // nothing outside css/ reads these fields. Clone the tree per stylesheet if that
            // ever matters.
            rule[PropertySymbol.parentStyleSheet] = styleSheet;
            rule[PropertySymbol.cssParser] = new CSSParser(styleSheet);
            if (Array.isArray(rule.cssRules)) rebind(rule.cssRules, window, styleSheet);
        }
    }
    CSSStyleSheet.prototype.replaceSync = function (text) {
        if (arguments.length === 0) {
            throw new this[PropertySymbol.window].TypeError(
                "Failed to execute 'replaceSync' on 'CSSStyleSheet': 1 argument required, but only 0 present.",
            );
        }
        let rules = CACHE.get(text);
        if (rules) {
            rebind(rules, this[PropertySymbol.window], this);
        } else {
            rules = new CSSParser(this).parseFromString(text);
            if (CACHE.size > 5000) CACHE.clear();
            CACHE.set(text, rules);
        }
        this.cssRules = rules;
    };
}

export default function patch(win) {
    // -----------------------------------------------------------------------------------
    // EventTarget.dispatchEvent — set globalThis.event during dispatch
    // Required for hx-vals="js:{...}" that reference the triggering event.
    // Public EventTarget differs from the internal prototype used by DOM nodes.
    // -----------------------------------------------------------------------------------
    {
        const _probe = win.document.createElement("div");
        let _etProto = Object.getPrototypeOf(_probe);
        while (_etProto && !Object.getOwnPropertyDescriptor(_etProto, "dispatchEvent"))
            _etProto = Object.getPrototypeOf(_etProto);
        if (_etProto) {
            const _origDispatch = _etProto.dispatchEvent;
            _etProto.dispatchEvent = function dispatchEvent(evt) {
                const prev = globalThis.event;
                globalThis.event = evt;
                try {
                    return _origDispatch.call(this, evt);
                } finally {
                    globalThis.event = prev;
                }
            };
        }
    }
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

    patchDomParser(win);
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
    // properties across at startup), so win.DOMParser would miss patchDomParser's
    // table-repair wrapper, which lives on globalThis.DOMParser.
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
