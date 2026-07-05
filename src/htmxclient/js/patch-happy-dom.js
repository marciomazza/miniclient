import patchURL from "./patch-happy-dom-url.js";
import patchDomParser from "./patch-happy-dom-parser.js";
import patchAttr from "./patch-happy-dom-attr.js";

function patchMethod(proto, method, wrapper) {
    const orig = proto[method];
    proto[method] = function (...args) {
        return wrapper.call(this, orig, ...args);
    };
}

export default function patch(win) {
    // -----------------------------------------------------------------------------------
    // history.pushState / replaceState — don't update location.href
    // -----------------------------------------------------------------------------------
    ["pushState", "replaceState"].forEach((method) => {
        const orig = win.history[method].bind(win.history);
        win.history[method] = function (state, title, url) {
            orig(state, title, url);
            if (url != null) win.location.href = String(url);
        };
    });

    // -----------------------------------------------------------------------------------
    // Element.matches — :disabled not propagated from <fieldset disabled>
    // -----------------------------------------------------------------------------------
    patchMethod(win.Element.prototype, "matches", function (_origMatches, selector) {
        const result = _origMatches.call(this, selector);
        if (!result && selector === ":disabled") {
            let p = this.parentElement;
            while (p) {
                if (p.tagName === "FIELDSET" && p.disabled) return true;
                p = p.parentElement;
            }
        }
        return result;
    });

    // -----------------------------------------------------------------------------------
    // HTMLElement.attachInternals — missing polyfill for form-associated custom elements
    // -----------------------------------------------------------------------------------
    patchMethod(win.HTMLElement.prototype, "attachInternals", function (_orig) {
        if (_orig) {
            try {
                return _orig.call(this);
            } catch {}
        }
        const host = this;
        return {
            setFormValue(val) {
                host.__internalsFormValue = val != null ? String(val) : null;
            },
        };
    });

    // -----------------------------------------------------------------------------------
    // Script execution — <script> elements not executed on DOM insertion
    // -----------------------------------------------------------------------------------
    {
        const _runScript = (s) => {
            if (s.textContent)
                try {
                    (0, eval)(s.textContent); // runs eval in global scope like an actual <script>
                } catch {}
        };
        const _evalScript = (el) => {
            if (el.nodeType !== Node.ELEMENT_NODE) return;
            if (el.tagName === "SCRIPT") _runScript(el);
            for (const s of el.querySelectorAll("script")) _runScript(s);
        };
        const _execScripts = (nodes) => {
            for (const n of nodes) if (n) _evalScript(n);
        };
        for (const m of ["replaceChildren", "append", "before", "after", "prepend"]) {
            patchMethod(win.Element.prototype, m, function (orig, ...nodes) {
                orig.call(this, ...nodes);
                _execScripts(nodes);
            });
        }
        // insertBefore is used by htmx morph when inserting unmatched new nodes into the DOM
        patchMethod(
            win.Node.prototype,
            "insertBefore",
            function (_origInsertBefore, newNode, refNode) {
                const result = _origInsertBefore.call(this, newNode, refNode);
                if (this.isConnected) _execScripts([newNode]);
                return result;
            },
        );
        // replaceWith is used during morph to swap out script nodes directly in the DOM
        patchMethod(win.Element.prototype, "replaceWith", function (_origReplaceWith, ...nodes) {
            const wasConnected = this.isConnected;
            _origReplaceWith.call(this, ...nodes);
            if (wasConnected) _execScripts(nodes);
        });
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
    // Element.innerHTML setter — two bugs after innerHTML parse:
    // (1) `selected` attr not reflected onto .selected IDL property
    // (2) radio mutual exclusion not enforced within a name group
    // -----------------------------------------------------------------------------------
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
                    this.querySelectorAll("option[selected]").forEach((opt) => {
                        opt.selected = true;
                    });
                    const groups = {};
                    this.querySelectorAll("input[type=radio]").forEach((r) => {
                        (groups[r.name] ??= []).push(r);
                    });
                    for (const group of Object.values(groups)) {
                        const checked = group.filter((r) => r.checked);
                        if (checked.length > 1)
                            checked.slice(0, -1).forEach((r) => {
                                r.checked = false;
                            });
                    }
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
    patchURL(win);
    patchDomParser(win);
    patchAttr(win);
}
