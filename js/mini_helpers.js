// @ts-check
// Bridge helpers with no per-navigation state (no closure over `win`/`dom`/host state), so they
// are defined once in the snapshot instead of rebuilt on every bootstrap.js restore.

// Resource-mount interceptor's content-type sniffing (bootstrap.js's resourceInterceptor).
/** @param {string} path */
export function contentTypeFor(path) {
    if (path.endsWith(".mjs") || path.endsWith(".js")) return "text/javascript";
    if (path.endsWith(".css")) return "text/css";
    if (path.endsWith(".json")) return "application/json";
    if (path.endsWith(".html")) return "text/html";
    return "application/octet-stream";
}

// Shared by fetch()'s own FormData handling (bootstrap.js) and submit.js's POST branch.
/** @param {FormData} formData */
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
/** @param {JSDOMWindow} win */
export function installXPathShortCircuit(win) {
    const HXON_INDEX_KEY = Symbol.for("miniclient.hxOnIndex");
    /** @param {string} expr */
    function hxOnShortCircuit(expr) {
        if (!expr.startsWith(".//*[@*[") || !expr.includes("starts-with(name()")) return null;
        const prefixes = [...expr.matchAll(/starts-with\(name\(\),\s*"([^"]*)"\)/g)].map(
            (m) => m[1],
        );
        const exacts = [...expr.matchAll(/(?:^|[\s([])@([a-z][\w:-]*)/gi)].map((m) => m[1]);
        /** @param {string} n */
        const matchName = (n) => prefixes.some((p) => n.startsWith(p)) || exacts.includes(n);
        /** @param {any} ctxWrapper */
        return function evaluate(ctxWrapper) {
            const ctx = globalThis.__jsdomImplForWrapper(ctxWrapper);
            const index = /** @type {any} */ (globalThis)[HXON_INDEX_KEY];
            /** @type {any[]} */
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
    const evaluatorProto = win.XPathEvaluator.prototype;
    const origCreateExpression = evaluatorProto.createExpression;
    /** @type {any} */ (evaluatorProto).createExpression = function (
        /** @type {string} */ expression,
        /** @type {XPathNSResolver | null | undefined} */ resolver,
    ) {
        const shortCircuit = hxOnShortCircuit(expression);
        if (shortCircuit) return { evaluate: shortCircuit };
        return origCreateExpression.call(/** @type {any} */ (this), expression, resolver);
    };
}

// jsdom's parser leaves multiple checked radios in one group checked when markup is
// assigned through innerHTML. Reapply the radio-group invariant after that parse; the
// checked setter already handles the sibling exclusion.
/** @param {JSDOMWindow} win */
export function installInnerHtmlRadioFix(win) {
    const innerHTML = /** @type {PropertyDescriptor} */ (
        Object.getOwnPropertyDescriptor(win.Element.prototype, "innerHTML")
    );
    const originalSet = /** @type {(v: string) => void} */ (innerHTML.set);
    Object.defineProperty(win.Element.prototype, "innerHTML", {
        get: innerHTML.get,
        /** @this {Element} @param {string} value */
        set(value) {
            originalSet.call(this, value);
            const groups = new Map();
            for (const radio of /** @type {NodeListOf<HTMLInputElement>} */ (
                this.querySelectorAll('input[type="radio"][name]')
            )) {
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
/** @param {JSDOMWindow} win */
export function installInnerText(win) {
    /** @param {HTMLElement} el */
    function isMiniHidden(el) {
        for (let node = /** @type {HTMLElement | null} */ (el); node; node = node.parentElement) {
            if (node.hidden) return true;
            const ownerWindow = node.ownerDocument?.defaultView;
            if (!ownerWindow) continue;
            const style = ownerWindow.getComputedStyle(node);
            if (style.display === "none" || style.visibility === "hidden") return true;
        }
        return false;
    }
    Object.defineProperty(win.HTMLElement.prototype, "innerText", {
        /** @this {HTMLElement} */
        get() {
            if (isMiniHidden(this)) return "";
            const ownerWindow = this.ownerDocument?.defaultView;
            if (!ownerWindow) return this.textContent;
            return globalThis.__innerText(this, {
                getComputedStyle: (/** @type {Element} */ el) => ownerWindow.getComputedStyle(el),
            });
        },
        /** @this {HTMLElement} @param {string} value */
        set(value) {
            this.textContent = value;
        },
        configurable: true,
    });
}
