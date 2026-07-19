// happy-dom's Document.createAttribute() always splits names on ":", even for plain
// (non-namespaced) setAttribute calls. That leaves the Attr with a prefix but no
// namespaceURI, which getNamedItemNS() refuses to resolve — so setNamedItem() can't
// find the "previous" attribute to replace and just appends instead of overwriting.
// getAttribute() then forever returns the first value ever set.
//
// This bug hits colon-containing names directly (e.g. hx-on:click), and also plain
// attrs that share a lookup slot with a colon-prefixed sibling (e.g. "style" next to
// ":style") — setting one can corrupt the other's slot too.
//
// Fix: drain existing entries for the affected name via removeAttribute() (which
// works correctly) before calling the original setAttribute(), so it always starts
// from a clean slate instead of piling up duplicates.
export default function patch(win) {
    const _origSetAttribute = win.Element.prototype.setAttribute;
    win.Element.prototype.setAttribute = function (name, value) {
        name = String(name);
        const bareName = name.startsWith(":") ? name.slice(1) : name;
        if (name.includes(":") || this.hasAttribute(":" + bareName)) {
            while (this.hasAttribute(name)) this.removeAttribute(name);
        }
        return _origSetAttribute.call(this, name, value);
    };
}
