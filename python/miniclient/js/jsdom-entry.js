// happy-dom's module graph required "buffer" eagerly, which is what actually replaced
// pre_globals.js's placeholder `globalThis.Buffer` with the real polyfill; jsdom's minimal
// construction path (empty HTML, no scripts) never reaches a require for it. Import it here,
// before jsdom, purely for that side effect -- ESM evaluates imports in declaration order, so
// this runs (and installs the real Buffer) before any of jsdom's own module graph does.
import "buffer";
import { JSDOM, requestInterceptor } from "jsdom";
import { implForWrapper, wrapperForImpl } from "jsdom/lib/generated/idl/utils.js";
import "./patch-jsdom-hxon-index.js";
// jsdom has no Request/Response of its own; undici (jsdom's own transitive dep) does.
// Importing its class files directly, not the `undici` package entry point, keeps its real
// network client/dispatcher out of the bundle -- bootstrap.js's fetch() only needs these as
// spec-compliant containers around mini's own op_fetch transport.
//
// Response must load first: request.js's module-top-level `RequestInit` webidl converter
// reads `webidl.converters.BodyInit`, which only exists once response.js's own module top
// level has run and assigned it.
import { Response } from "undici/lib/web/fetch/response.js";
import { Request } from "undici/lib/web/fetch/request.js";
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
// scoped to it -- this is the one explicit hand-off point to the shared realm. Everywhere
// else (bootstrap.js, snapshot_warmup.js, ...) just references the bare `JSDOM` global.
globalThis.JSDOM = JSDOM;
globalThis.Request = Request;
globalThis.Response = Response;
globalThis.__jsdomRequestInterceptor = requestInterceptor;

// bootstrap.js runs as a classic script, never a module, so it can't `import` jsdom's
// webidl2js wrapper<->impl converters directly -- needed there to bridge the public Node
// the hx-on index short-circuit receives to the impl objects patch-jsdom-hxon-index.js
// indexes.
globalThis.__jsdomImplForWrapper = implForWrapper;
globalThis.__jsdomWrapperForImpl = wrapperForImpl;
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
