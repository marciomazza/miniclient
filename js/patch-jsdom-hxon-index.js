// htmx's #hxOnQuery and hx-live's bind scan both run `.//*[@*[starts-with(name(), ...)]]`
// on every process() -- an XPath walk over every element x every attribute name of a
// subtree. Keep a grow-only Set of elements carrying an attribute name one of those queries
// could select; the patched XPathEvaluator (bootstrap.js) short-circuits them against it. A
// superset is safe: the evaluator re-filters by subtree membership and the exact name
// predicate, and htmx's __handleHxOnAttributes is idempotent. Detached nodes are pruned
// lazily by the evaluator's isConnected check.
//
// One Set per isolate (globalThis, not per-document): a <template>'s content briefly lives
// in a separate "template contents owner document" (its elements' ownerDocument at
// attribute-set time), then moves into the real document on insertion -- htmx's own
// swap/makeFragment path does exactly this. A per-document Set would index the element under
// the template's throwaway document and never find it again once adopted; a single Set
// keyed on nothing survives that move for free. Every Runtime owns its own isolate, so this
// still dies with the Window/Runtime -- just at process-teardown grain instead of navigation
// grain.
//
// jsdom has no PropertySymbol-style per-Window attribute hook: the one choke point every
// attribute write goes through -- public setAttribute(NS)/setAttributeNode(NS)/
// toggleAttribute, the parse5-backed HTML parser, and cloneNode -- is
// ElementImpl.prototype._attributeChangeSteps(localName, oldValue, value, namespace)
// (value === null on removal). An HTML attribute has no namespace, so its local name is the
// whole name, colons included.
// This impl class is a CommonJS singleton shared by every Window in the process, so the
// patch is applied once at import time instead of per-navigation.
import { implementation as ElementImpl } from "jsdom/lib/jsdom/living/nodes/Element-impl.js";

// CONTRACT: same registry string as bootstrap.js's XPathEvaluator short-circuit, which is
// the only reader. Keep in sync.
export const HXON_INDEX_KEY = Symbol.for("miniclient.hxOnIndex");

// The index must be a superset of every attribute name the recognised XPath queries can
// select. The real prefixes derive from htmx.config (loaded after this runs), so this
// assumes defaults plus a `hx:` bindPrefix.
// A consumer that sets a non-default `prefix`, or a `live.bindPrefix` outside {`:`, `hx:`},
// silently gets a stale index and its hx-on / hx-live handlers won't bind. Kept this narrow
// on purpose -- widening to all `hx-*` roughly halves the win, because the evaluator's
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

const origAttributeChangeSteps = ElementImpl.prototype._attributeChangeSteps;
ElementImpl.prototype._attributeChangeSteps = function (localName, oldValue, value, namespace) {
    if (value !== null && localName && interesting(localName)) {
        (globalThis[HXON_INDEX_KEY] ??= new Set()).add(this);
    }
    return origAttributeChangeSteps.call(this, localName, oldValue, value, namespace);
};
