// Element.setAttribute — colon-containing attribute names (e.g. hx-on:click) never
// update their value past the first call.
//
// happy-dom's Document.createAttribute() always splits the name on ":" into a
// prefix/localName pair, even for plain (non-namespaced) setAttribute calls, where the
// DOM spec says the qualified name must be kept whole with no namespace and no prefix.
// The resulting Attr ends up with a prefix but no namespaceURI — a combination
// NamedNodeMap#getNamedItemNS() (correctly) refuses to resolve, since real namespaced
// attributes always have both. That makes setNamedItem() unable to find the "previous"
// attribute to replace, so it keeps appending instead of overwriting: getAttribute()
// (which reads index 0) forever returns the very first value ever set.
//
// Fix: before assigning a colon-named attribute, drain any existing entries for that
// name via the (correctly working) removeAttribute() path, so setAttribute() always
// starts from a clean slate instead of piling up stale duplicates.
export default function patch(win) {
    const _origSetAttribute = win.Element.prototype.setAttribute;
    win.Element.prototype.setAttribute = function (name, value) {
        if (String(name).includes(":")) {
            while (this.hasAttribute(name)) this.removeAttribute(name);
        }
        return _origSetAttribute.call(this, name, value);
    };
}
