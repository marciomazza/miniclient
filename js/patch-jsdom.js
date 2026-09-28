import { patchElementInternals } from "./patch-jsdom-element-internals.js";

// jsdom has DOMParser (including text/html) but not the Declarative Shadow DOM family of
// static Document methods -- see https://github.com/jsdom/jsdom/issues/3955 (open; a PR
// implementing getHTML() was closed unmerged). Declarative shadow roots themselves stay
// unsupported here too: jsdom's own HTML parser has no <template shadowrootmode> handling
// for parseHTMLUnsafe() to plug into.
function patchParseHTMLUnsafe(win) {
    Object.defineProperty(win.Document, "parseHTMLUnsafe", {
        value(html) {
            return new win.DOMParser().parseFromString(`${html}`, "text/html");
        },
        writable: true,
        configurable: true,
    });
}

// jsdom recreates these classes per Window (unlike patch-jsdom-hxon-index.js's singleton
// Element-impl patch), so each patch below must run once per Window -- bootstrap.js calls
// this after every Window is constructed.
export function patchJsdom(win) {
    patchParseHTMLUnsafe(win);
    patchElementInternals(win);
}
