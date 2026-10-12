// @ts-check
// Runs last in the snapshot build: builds the blank-boot Window once, so a boot at that URL only
// attaches host state (see bootstrap.js) instead of constructing ~150 interface classes again.
// Left unparsed (`deferParse`): a parsed document holds WeakRefs, which the serializer rejects.
// `url` must equal BLANK_URL in src/runtime.rs.
(() => {
    const url = "http://localhost/";
    const cookieJar = new globalThis.__JSDOMCookieJar();
    /** @type {any} */ (globalThis).__BAKED__ = {
        url,
        cookieJar,
        dom: globalThis.__mini.createDom(url, cookieJar, true),
    };
})();
