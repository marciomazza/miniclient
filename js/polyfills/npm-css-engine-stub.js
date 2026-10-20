// stylo owns selector matching and CSS parsing, so dom-selector and css-tree stay out of the
// bundle. jsdom still imports them from two dead paths: Document-impl's lazy DOMSelector and
// CSS-impl, whose CSS namespace pre_globals.js replaces. Both throw if a fork change revives them.
const _removed = () => {
    throw new Error("dom-selector and css-tree are not bundled; stylo owns CSS in this runtime");
};

class DOMSelector {
    constructor() {
        _removed();
    }
}

class CSSImpl {
    static escape() {
        _removed();
    }

    static supports() {
        _removed();
    }
}

module.exports = { DOMSelector, implementation: CSSImpl };
