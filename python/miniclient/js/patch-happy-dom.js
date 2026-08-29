import patchDomParser from "./patch-happy-dom-parser.js";
import patchAttr from "./patch-happy-dom-attr.js";
import SyncFetchScriptBuilder from "happy-dom/lib/fetch/utilities/SyncFetchScriptBuilder.js";
import SelectorItem from "happy-dom/lib/query-selector/SelectorItem.js";
import SelectorParser from "happy-dom/lib/query-selector/SelectorParser.js";
import * as PropertySymbol from "happy-dom/lib/PropertySymbol.js";

function patchMethod(proto, method, wrapper) {
    const orig = proto[method];
    proto[method] = function (...args) {
        return wrapper.call(this, orig, ...args);
    };
}

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

export default function patch(win) {
    // -----------------------------------------------------------------------------------
    // Location.hash setter — happy-dom's own setter pushes a new history entry but carries
    // the *previous* entry's state forward instead of nulling it. Per spec, a script-driven
    // hash-only navigation always gets a fresh entry with state: null (only explicit
    // pushState/replaceState calls carry a state object). Delegate to history.pushState,
    // which already creates a fresh entry with an explicit state and updates the URL
    // (including firing hashchange) the same way the original setter did.
    // -----------------------------------------------------------------------------------
    {
        const locProto = Object.getPrototypeOf(win.location);
        const desc = Object.getOwnPropertyDescriptor(locProto, "hash");
        Object.defineProperty(locProto, "hash", {
            get: desc.get,
            set(hash) {
                const url = new URL(this.href);
                url.hash = hash;
                if (url.hash !== this.hash) win.history.pushState(null, "", url.href);
            },
            configurable: true,
        });
    }

    // -----------------------------------------------------------------------------------
    // SelectorItem.matchPseudoItem — happy-dom gaps in constraint/disabled pseudo-classes:
    // - :disabled doesn't propagate from an ancestor <fieldset disabled>
    // - :required/:invalid/:valid have no case in the switch at all
    // Patched here (not Element.matches) so :has(), :is(), :not(), and querySelectorAll()
    // see the fix too, not just direct .matches() calls.
    // -----------------------------------------------------------------------------------
    patchMethod(
        SelectorItem.prototype,
        "matchPseudoItem",
        function (_orig, scope, element, parentChildren, pseudo, ignoreErrors) {
            const result = _orig.call(this, scope, element, parentChildren, pseudo, ignoreErrors);
            if (result) return result;
            if (pseudo.name === "disabled") {
                let p = element.parentElement;
                while (p) {
                    if (p.tagName === "FIELDSET" && p.disabled) return { priorityWeight: 10 };
                    p = p.parentElement;
                }
                return null;
            }
            if (pseudo.name === "required") {
                return element.hasAttribute?.("required") ? { priorityWeight: 10 } : null;
            }
            if (pseudo.name === "invalid" || pseudo.name === "valid") {
                if (typeof element.checkValidity !== "function") return null;
                const valid = element.checkValidity();
                return (pseudo.name === "invalid" ? !valid : valid) ? { priorityWeight: 10 } : null;
            }
            return null;
        },
    );

    // -----------------------------------------------------------------------------------
    // :checked pseudo-class — only matches <input>, but per spec it also applies to a
    // selected <option> within a <select>. Unlike :disabled above, querySelectorAll
    // reaches this through SelectorItem.matchPseudoItem directly (QuerySelector.findAll
    // calls selectorItem.match()), bypassing the patchable public Element.prototype.matches.
    // -----------------------------------------------------------------------------------
    patchMethod(
        SelectorItem.prototype,
        "matchPseudoItem",
        function (_origMatchPseudoItem, scope, element, parentChildren, pseudo, ignoreErrors) {
            if (pseudo.name === "checked" && element.tagName === "OPTION") {
                return element.selected ? { priorityWeight: 10 } : null;
            }
            return _origMatchPseudoItem.call(
                this,
                scope,
                element,
                parentChildren,
                pseudo,
                ignoreErrors,
            );
        },
    );

    // -----------------------------------------------------------------------------------
    // HTMLSelectElement.value setter — sets each <option>'s internal selectedness symbol
    // directly, bypassing HTMLOptionElement's own `selected` setter entirely. Unlike
    // HTMLInputElement's #setChecked (which does call [clearCache]() after flipping
    // `checked`), this leaves any previously-cached querySelectorAll/matches/querySelector
    // result that depends on `:checked` (e.g. `option:checked`) stale forever, since the
    // cache is keyed by selector string and invalidated per-node via [clearCache](), which
    // nothing here ever calls.
    // -----------------------------------------------------------------------------------
    {
        const selectProto = Object.getPrototypeOf(win.document.createElement("select"));
        const desc = Object.getOwnPropertyDescriptor(selectProto, "value");
        Object.defineProperty(selectProto, "value", {
            get: desc.get,
            set(value) {
                desc.set.call(this, value);
                for (const option of this.querySelectorAll("option")) {
                    option[PropertySymbol.clearCache]();
                }
            },
            configurable: true,
        });
    }

    // -----------------------------------------------------------------------------------
    // HTMLFormElement.reset — SELECT handling ignores `element.multiple` entirely: per
    // spec, "if no option has a `selected` attribute, auto-select the first option" only
    // applies to single-selects. A `<select multiple>` with no selected-attribute options
    // must end up with nothing selected, but happy-dom force-selects options[0] regardless.
    // Also fixes a latent bug in the same branch: it only honors the *first* selected-
    // attribute option, so a multi-select with several pre-selected options lost all but
    // one. Runs the original reset() first (TEXTAREA/INPUT/OUTPUT handling stays correct),
    // then overwrites SELECT selectedness with the spec-correct logic.
    // -----------------------------------------------------------------------------------
    patchMethod(win.HTMLFormElement.prototype, "reset", function (_orig) {
        _orig.call(this);
        for (const element of this[PropertySymbol.getFormControlItems]()) {
            if (element.tagName !== "SELECT") continue;
            const options = [...element.options];
            if (element.multiple) {
                for (const option of options) option.selected = option.hasAttribute("selected");
            } else {
                const selectedOptions = options.filter((o) => o.hasAttribute("selected"));
                const toSelect =
                    selectedOptions.length > 0
                        ? selectedOptions[selectedOptions.length - 1]
                        : options[0];
                for (const option of options) option.selected = option === toSelect;
            }
        }
    });

    // -----------------------------------------------------------------------------------
    // HTMLElement.attachInternals — missing polyfill for form-associated custom elements
    // -----------------------------------------------------------------------------------
    patchMethod(win.HTMLElement.prototype, "attachInternals", function (_orig) {
        if (_orig) {
            return _orig.call(this);
        }
        const host = this;
        return {
            setFormValue(val) {
                host.__internalsFormValue = val != null ? String(val) : null;
            },
        };
    });

    // -----------------------------------------------------------------------------------
    // HTMLFormElement[getFormControlItems] — form-associated custom elements are "listed"
    // per spec and belong in form.elements, but happy-dom's query only covers
    // input/select/textarea/button/fieldset/object/output. Without this, form.elements
    // (and anything built from it, e.g. htmx's __collectFormData dedup set) treats such
    // elements as absent from the form.
    // -----------------------------------------------------------------------------------
    {
        const _probe = win.document.createElement("form");
        let _formProto = Object.getPrototypeOf(_probe);
        while (
            _formProto &&
            !Object.getOwnPropertyDescriptor(_formProto, PropertySymbol.getFormControlItems)
        )
            _formProto = Object.getPrototypeOf(_formProto);
        if (_formProto) {
            patchMethod(_formProto, PropertySymbol.getFormControlItems, function (_orig) {
                const items = _orig.call(this);
                for (const el of this.querySelectorAll("*")) {
                    if (typeof el.__internalsFormValue !== "undefined" && !items.includes(el)) {
                        items.push(el);
                    }
                }
                return items;
            });
        }
    }

    // -----------------------------------------------------------------------------------
    // document.getElementById — doesn't respect tree order with duplicate IDs
    // (e.g. when htmx stores a preserved element in a pantry node after <body>)
    // -----------------------------------------------------------------------------------
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

    // -----------------------------------------------------------------------------------
    // Post-parse fixups — two bugs after happy-dom parses HTML (via innerHTML setter or
    // document.write):
    // (1) `selected` attr not reflected onto .selected IDL property
    // (2) radio mutual exclusion not enforced within a name group
    // -----------------------------------------------------------------------------------
    globalThis.__zzz_fixup_parsed_dom = function (root) {
        root.querySelectorAll("option[selected]").forEach((opt) => {
            opt.selected = true;
        });
        const groups = {};
        root.querySelectorAll("input[type=radio]").forEach((r) => {
            (groups[r.name] ??= []).push(r);
        });
        for (const group of Object.values(groups)) {
            const checked = group.filter((r) => r.checked);
            if (checked.length > 1)
                checked.slice(0, -1).forEach((r) => {
                    r.checked = false;
                });
        }
    };
    {
        const _probe = win.document.createElement("div");
        let _elProto = Object.getPrototypeOf(_probe);
        while (_elProto && !Object.getOwnPropertyDescriptor(_elProto, "innerHTML"))
            _elProto = Object.getPrototypeOf(_elProto);
        if (_elProto) {
            const _desc = Object.getOwnPropertyDescriptor(_elProto, "innerHTML");
            Object.defineProperty(_elProto, "innerHTML", {
                get: _desc.get,
                set(value) {
                    _desc.set.call(this, value);
                    globalThis.__zzz_fixup_parsed_dom(this);
                },
                configurable: true,
            });
        }
    }

    // -----------------------------------------------------------------------------------
    // HTMLTextAreaElement.value getter — when not dirty, must return the "child text
    // content" (direct Text-node children only), but happy-dom returns the full
    // recursive textContent instead. This matters because htmx can insert element
    // children into a textarea via DOM APIs (bypassing the HTML parser's RCDATA
    // restriction), in which case only direct text children should count.
    // -----------------------------------------------------------------------------------
    {
        const _desc = Object.getOwnPropertyDescriptor(win.HTMLTextAreaElement.prototype, "value");
        Object.defineProperty(win.HTMLTextAreaElement.prototype, "value", {
            get() {
                const value = _desc.get.call(this);
                if (value !== this.textContent) return value;
                let text = "";
                for (const child of this.childNodes)
                    if (child.nodeType === Node.TEXT_NODE) text += child.data;
                return text;
            },
            set: _desc.set,
            configurable: true,
        });
    }

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
    // -----------------------------------------------------------------------------------
    // SyncFetchScriptBuilder.getScript — replace the "spawn node -e <script>" script
    // generation with a plain envelope object for our node:child_process polyfill's
    // execFileSync (see node-child-process.js). Both ends are our own code passing an
    // in-heap object, so no serialization is needed here (unlike the real subprocess
    // this used to emulate, which had to shuttle everything through a text pipe).
    // -----------------------------------------------------------------------------------
    patchMethod(SyncFetchScriptBuilder, "getScript", function (_orig, request) {
        return {
            __sync_fetch__: true,
            url: request.url.href,
            method: request.method,
            headers: request.headers ?? {},
            body: request.body ?? null,
        };
    });

    // -----------------------------------------------------------------------------------
    // Node[connectedToNode] — for node types whose constructor returns a Proxy standing in
    // for `this` (HTMLFormElement, HTMLSelectElement — needed for named-item access like
    // form.username), the proxy's `get` trap permanently binds every symbol-keyed method to
    // the raw target the first time it's read. connectedToNode then stamps
    // `childNodes[i][parentNode] = this` using that raw target instead of the canonical
    // proxy, so a child's `.parentElement` and the parent's own `.querySelector()`/
    // `.firstChild` end up disagreeing about node identity after any move (appendChild/
    // insertBefore). Force `this` back to the proxy (if any) before delegating, mirroring
    // how appendChild/insertBefore/removeChild already resolve
    // `self = this[PropertySymbol.proxy] || this` elsewhere in happy-dom's own Node.js.
    // -----------------------------------------------------------------------------------
    patchMethod(win.Node.prototype, PropertySymbol.connectedToNode, function (_orig) {
        return _orig.call(this[PropertySymbol.proxy] || this);
    });

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
    patchAttr(win);

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
