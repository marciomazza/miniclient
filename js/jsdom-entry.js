// Some modules in the bundle (notably undici) require "buffer" eagerly at module top
// level. Importing it here installs the real polyfill on globalThis.Buffer before any
// of those modules do their require resolution; ESM evaluates imports in declaration
// order, so this runs first.
import "buffer";
import { CookieJar, JSDOM } from "jsdom";
import { implForWrapper, wrapperForImpl } from "jsdom/lib/generated/idl/utils.js";
import { serializeURL } from "whatwg-url";
import "./patch-jsdom-hxon-index.js";
import { patchJsdom } from "./patch-jsdom.js";
// jsdom has no DataTransfer/innerText of its own -- both ported from the jsdom-ecosystem
// (not reimplemented) since real, tested implementations of these already exist. Only the
// factory function is imported, not the whole @testing-library/user-event package: the
// rest of it (event simulation) is out of scope, and this deep import keeps
// @testing-library/dom (its peer dep) out of the bundle entirely.
import { createDataTransfer } from "@testing-library/user-event/dist/esm/utils/dataTransfer/DataTransfer.js";
import innerText from "styleless-innertext";
// user-event's own `.files` (FileList.js, unused here) fakes a FileList with a
// prototype swap, which real jsdom rejects: `input.files = ...` runs through jsdom's
// webidl2js conversion, which brand-checks an internal symbol no amount of prototype
// spoofing can add. A real, convertible FileList only comes from jsdom's own generated
// factory -- same one jsdom's own HTMLInputElement-impl.js uses internally.
import { create as createFileListWrapper } from "jsdom/lib/generated/idl/FileList.js";

// The esbuild bundle wraps this entry point in an anonymous IIFE, so the import above stays
// scoped to it -- this is the one explicit hand-off point to the shared realm.
globalThis.JSDOM = JSDOM;
globalThis.__JSDOMCookieJar = CookieJar;
globalThis.__serializeURL = serializeURL;

// bootstrap.js runs as a classic script, never a module, so it can't `import` jsdom's
// webidl2js wrapper<->impl converters directly -- needed there to bridge the public Node
// the hx-on index short-circuit receives to the impl objects patch-jsdom-hxon-index.js
// indexes.
globalThis.__jsdomImplForWrapper = implForWrapper;
globalThis.__jsdomWrapperForImpl = wrapperForImpl;
globalThis.__patchJsdom = patchJsdom;
globalThis.__innerText = innerText;
// user-event's own `.files` getter (default files=[], fixed at construction) is replaced
// with one derived live from `.items` -- see bootstrap.js's DataTransfer polyfill.
globalThis.__createDataTransfer = createDataTransfer;
globalThis.__createRealFileList = function (win, fileWrappers) {
    const wrapper = createFileListWrapper(win, [], {});
    const impl = implForWrapper(wrapper);
    for (const f of fileWrappers) impl.push(f);
    return wrapper;
};
