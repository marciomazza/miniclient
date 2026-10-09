// @ts-check
// Bridge helpers with no per-navigation state (no closure over `win`/`dom`/host state), so they
// are defined once in the snapshot instead of rebuilt on every bootstrap.js restore.

// Resource-mount interceptor's content-type sniffing (bootstrap.js's resourceInterceptor).
export function contentTypeFor(path) {
    if (path.endsWith(".mjs") || path.endsWith(".js")) return "text/javascript";
    if (path.endsWith(".css")) return "text/css";
    if (path.endsWith(".json")) return "application/json";
    if (path.endsWith(".html")) return "text/html";
    return "application/octet-stream";
}

// Shared by fetch()'s own FormData handling (bootstrap.js) and submit.js's POST branch.
export function serializeMultipart(formData) {
    const boundary = "----HxClientBoundary" + Math.random().toString(36).slice(2, 18);
    let body = "";
    for (const [name, value] of formData) {
        body += `--${boundary}\r\nContent-Disposition: form-data; name="${name}"\r\n\r\n${value}\r\n`;
    }
    body += `--${boundary}--`;
    return { boundary, body };
}

// Per-Window jsdom polyfills, installed by bootstrap.js's registerWindowGlobals on every
// navigation. Each takes the new `win` as a parameter rather than closing over one, so
// they're reusable across every Window instance instead of being redefined per restore.

// XPathEvaluator.prototype.createExpression — short-circuit the two XPath shapes htmx
// actually compiles (#hxOnQuery and hx-live's bind scan, both
// `.//*[@*[starts-with(name(), "...") or @exact]]`) against patch-jsdom-hxon-index.js's
// grow-only Set instead of walking the real XPath engine over the whole subtree on every
// process(). A superset match is safe here (see that file's header); this is the only
// reader of its Set, kept on globalThis (not per-document -- see that file's header for
// why). win.XPathEvaluator is a fresh class per Window, so this must be reapplied every
// navigation like the ANY_TYPE patch in bootstrap.js.
export function installXPathShortCircuit(win) {
    const HXON_INDEX_KEY = Symbol.for("miniclient.hxOnIndex");
    function hxOnShortCircuit(expr) {
        if (!expr.startsWith(".//*[@*[") || !expr.includes("starts-with(name()")) return null;
        const prefixes = [...expr.matchAll(/starts-with\(name\(\),\s*"([^"]*)"\)/g)].map(
            (m) => m[1],
        );
        const exacts = [...expr.matchAll(/(?:^|[\s([])@([a-z][\w:-]*)/gi)].map((m) => m[1]);
        const matchName = (n) => prefixes.some((p) => n.startsWith(p)) || exacts.includes(n);
        return function evaluate(ctxWrapper) {
            const ctx = globalThis.__jsdomImplForWrapper(ctxWrapper);
            const index = globalThis[HXON_INDEX_KEY];
            const out = [];
            if (index) {
                for (const el of index) {
                    if (!el.isConnected) {
                        index.delete(el);
                        continue;
                    }
                    if (ctx !== el && !ctx.contains(el)) continue;
                    if (el.getAttributeNames().some(matchName))
                        out.push(globalThis.__jsdomWrapperForImpl(el));
                }
            }
            let i = 0;
            return { iterateNext: () => out[i++] ?? null };
        };
    }
    const origCreateExpression = win.XPathEvaluator.prototype.createExpression;
    win.XPathEvaluator.prototype.createExpression = function (expression, resolver) {
        const shortCircuit = hxOnShortCircuit(expression);
        if (shortCircuit) return { evaluate: shortCircuit };
        return origCreateExpression.call(this, expression, resolver);
    };
}

// jsdom's parser leaves multiple checked radios in one group checked when markup is
// assigned through innerHTML. Reapply the radio-group invariant after that parse; the
// checked setter already handles the sibling exclusion.
export function installInnerHtmlRadioFix(win) {
    const innerHTML = Object.getOwnPropertyDescriptor(win.Element.prototype, "innerHTML");
    const originalSet = innerHTML.set;
    Object.defineProperty(win.Element.prototype, "innerHTML", {
        get: innerHTML.get,
        set(value) {
            originalSet.call(this, value);
            const groups = new Map();
            for (const radio of this.querySelectorAll('input[type="radio"][name]')) {
                if (!radio.name || !radio.checked) continue;
                const byName = groups.get(radio.form ?? radio.getRootNode()) ?? new Map();
                const previous = byName.get(radio.name);
                if (previous) previous.checked = false;
                byName.set(radio.name, radio);
                groups.set(radio.form ?? radio.getRootNode(), byName);
            }
        },
        configurable: true,
    });
}

// innerText: jsdom has no layout engine, so it never implements this (needs rendered-box
// information). styleless-innertext (ported, not reimplemented) approximates it from the
// DOM tree + computed style instead of real layout. Its own algorithm has no hidden-element
// exclusion, so a self/ancestor `hidden` attribute or display:none/visibility:hidden check
// is added here; hidden *descendants* still leak their text, an accepted gap since real
// layout info doesn't exist to detect them either.
// DOMParser-created documents have no browsing context; jsdom rejects cross-window style
// lookup, so detached fragments use textContent instead.
export function installInnerText(win) {
    function isMiniHidden(el) {
        for (let node = el; node; node = node.parentElement) {
            if (node.hidden) return true;
            const ownerWindow = node.ownerDocument?.defaultView;
            if (!ownerWindow) continue;
            const style = ownerWindow.getComputedStyle(node);
            if (style.display === "none" || style.visibility === "hidden") return true;
        }
        return false;
    }
    Object.defineProperty(win.HTMLElement.prototype, "innerText", {
        get() {
            if (isMiniHidden(this)) return "";
            const ownerWindow = this.ownerDocument?.defaultView;
            if (!ownerWindow) return this.textContent;
            return globalThis.__innerText(this, {
                getComputedStyle: (el) => ownerWindow.getComputedStyle(el),
            });
        },
        set(value) {
            this.textContent = value;
        },
        configurable: true,
    });
}
