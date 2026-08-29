// htmx's #hxOnQuery and hx-live's bind scan both run `.//*[@*[starts-with(name(), ...)]]`
// on every process() — an XPath walk over every element × every attribute name of a
// subtree. Keep a grow-only per-document Set of elements carrying an attribute name one of
// those queries could select; the patched XPathEvaluator (pre_globals.js) short-circuits
// them against it. A superset is safe: the evaluator re-filters by subtree membership and
// the exact name predicate, and htmx's __handleHxOnAttributes is idempotent. Keyed on the
// document so it dies with the Window on navigation; detached nodes are pruned lazily by
// the evaluator's isConnected check.
import * as PropertySymbol from "happy-dom/lib/PropertySymbol.js";

// CONTRACT: same registry string as pre_globals.js's XPathEvaluator short-circuit,
// which is the only reader. Not imported there (that file isn't a module). Keep in sync.
export const HXON_INDEX_KEY = Symbol.for("miniclient.hxOnIndex");

// The index must be a superset of every attribute name the recognised XPath queries can
// select: htmx's #hxOnQuery matches `hx-on*` / `data-hx-on*`, hx-live's scan matches
// `hx-live*` / `data-hx-live*` and its bindPrefix. The real prefixes derive from
// htmx.config (`prefix` default `data-hx-`, `live.bindPrefix` default `:` — `hx:` in
// eventos, `metaCharacter` default `:`), which isn't loaded this early: the index is built
// during HTML parse, before htmx. So this assumes those defaults plus a `hx:` bindPrefix.
// A consumer that sets a non-default `prefix`, or a `live.bindPrefix` outside {`:`, `hx:`},
// silently gets a stale index and its hx-on / hx-live handlers won't bind. Kept this narrow
// on purpose — widening to all `hx-*` roughly halves the win, because the evaluator's
// filter loop then walks every hx-get/hx-post element too.
function interesting(name) {
    return (
        name.charCodeAt(0) === 58 /* ":" */ ||
        name.startsWith("hx-on") ||
        name.startsWith("data-hx-on") ||
        name.startsWith("hx-live") ||
        name.startsWith("data-hx-live") ||
        name.startsWith("hx:")
    );
}

export default function patchHxOnIndex(win) {
    // Load-order tripwire: the reader is installed by pre_globals.js at snapshot time.
    if (typeof globalThis.XPathEvaluator !== "function") {
        throw new Error("patchHxOnIndex: XPathEvaluator polyfill missing (pre_globals.js)");
    }
    const proto = win.Element.prototype;
    const origOnSet = proto[PropertySymbol.onSetAttribute];
    proto[PropertySymbol.onSetAttribute] = function (attribute, replacedAttribute) {
        const name = attribute[PropertySymbol.name];
        if (name && interesting(name)) {
            const doc = this.ownerDocument;
            if (doc) (doc[HXON_INDEX_KEY] ??= new Set()).add(this);
        }
        return origOnSet.call(this, attribute, replacedAttribute);
    };
}
