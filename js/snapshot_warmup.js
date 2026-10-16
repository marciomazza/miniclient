// @ts-check
// Second pass of deno_core's two-pass snapshot: this runs against the cold snapshot, and the
// context it exercises is what ships. The snapshot is built with eager compilation, so this
// is not about compiling code; it builds the lazily initialised state (regexps, caches) that
// every Page would otherwise pay for first -- window construction, jsdom's HTML parser, DOM
// manipulation and traversal, and FormData. Sheets are left out: the style engine's ops exist
// only at runtime. It ships in production, so it must never touch anything from tests/.
//
// `Event` (and friends) come off `win`, not `globalThis`: bootstrap.js's global registration
// -- the step that normally copies window properties onto globalThis -- never runs during a
// snapshot build, only scripts.
//
// What it leaves out is dictated by V8's snapshot serializer, which segfaults on a warmed
// context that ever constructed a `WeakRef`. htmx is out for a different reason: it is not in
// the snapshot at all, a page loads it from a <script> tag. The hx-* attributes below still
// warm the parser path htmx's own scan walks.
//
// jsdom's native `XPathEvaluator` is deliberately not exercised here either: unlike the old
// npm `xpath` package it replaced, its implementation trips the same WeakRef/cppgc serializer
// abort. It gets JIT-compiled on first real use per Page instead of during warmup.
//
// The Rust tests `runtime_scripts_and_warmup_produce_a_bootable_snapshot` and
// `appended_scripts_produce_a_distinct_bootable_snapshot` are the guard: they fail if
// anything added here stops the warmed context from serializing.
(() => {
    const dom = new JSDOM("", { url: "about:blank" });
    const win = dom.window;
    const doc = win.document;

    const root = doc.createElement("div");
    root.innerHTML =
        '<button hx-get="/warm" hx-target="#warm">go</button>' +
        '<form><input name="a" value="1"></form>';
    const first = /** @type {Element} */ (root.firstElementChild);
    first.getAttribute("hx-get");

    // DOM mutation: the other half of every Page's hot path, alongside parsing.
    const span = doc.createElement("span");
    root.appendChild(span);
    root.insertBefore(doc.createElement("i"), span);
    root.replaceChild(doc.createElement("b"), span);
    root.removeChild(/** @type {Element} */ (root.lastElementChild));
    root.cloneNode(true);

    root.setAttribute("data-warm", "1");
    root.removeAttribute("data-warm");
    root.classList.add("warm");
    root.classList.toggle("warm");
    root.classList.remove("warm");
    root.textContent = "warm";

    root.addEventListener("click", () => {});
    root.dispatchEvent(new win.Event("click", { bubbles: true }));
    root.matches("div");
    root.closest("div");

    const form = new win.FormData();
    form.append("a", "1");
    [...form.entries()];

    dom.window.close();
})();

// Cookies: tough-cookie's first parse/match/serialise in a fresh isolate otherwise compiles cold
// (~1.5 ms for the first `document.cookie` set). Covers the paths a Page takes: document.cookie
// set and get, Set-Cookie with Expires and Max-Age, the dispatcher's per-hop string read, and the
// serialise/deserialise that carries the jar across a navigation.
(() => {
    const Jar = globalThis.__JSDOMCookieJar;
    const jar = new Jar();
    const dom = new JSDOM("", { url: "http://localhost/a/b", cookieJar: jar });
    const doc = dom.window.document;
    doc.cookie = "sessionid=abcdef0123456789; path=/";
    doc.cookie = "w=1; Max-Age=3600; SameSite=Lax; Secure";
    jar.setCookieSync(
        "csrftoken=abc; expires=Wed, 08 Oct 2037 10:00:00 GMT; Max-Age=31449600; Path=/; HttpOnly",
        "http://localhost/a/b",
        { ignoreError: true },
    );
    doc.cookie;
    jar.getCookieStringSync("http://localhost/x");
    Jar.deserializeSync(/** @type {any} */ (jar.serializeSync()), new Jar().store);
    dom.window.close();
})();

globalThis.__clearDomBindings();
delete (/** @type {any} */ (globalThis).__clearDomBindings);
