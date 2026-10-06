// Second pass of deno_core's two-pass snapshot: this runs against the cold snapshot, and the
// context it exercises is what ships, so whatever it compiles is already compiled at Page
// load. It warms what every Page pays for first -- window construction, jsdom's HTML parser,
// DOM manipulation and traversal, FormData, and the CSS stack (stylesheet parse, selectors,
// computed style). It ships in production, so it must never
// touch anything from tests/.
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
    root.firstElementChild.getAttribute("hx-get");

    // DOM mutation: the other half of every Page's hot path, alongside parsing.
    const span = doc.createElement("span");
    root.appendChild(span);
    root.insertBefore(doc.createElement("i"), span);
    root.replaceChild(doc.createElement("b"), span);
    root.removeChild(root.lastElementChild);
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

// CSS: the first stylesheet read in a fresh isolate otherwise compiles css-tree, dom-selector
// and css-color cold (~75 ms).
(() => {
    const dom = new JSDOM(
        "<style>" +
            ".a, #b > .c:not(.d) { color: rgb(1 2 3 / .5); background: #abc; margin: 1px 2em }" +
            "@media (min-width: 1px) { div[data-x^='y'] { border: 1px solid hsl(10 20% 30%) } }" +
            "a:hover::before { content: 'x'; color: color-mix(in srgb, red 40%, blue) }" +
            "</style><div id=b><p class='c a' data-x=y1>t</p></div>",
        { url: "about:blank" },
    );
    const win = dom.window;
    const doc = win.document;
    const rules = doc.styleSheets[0].cssRules; // css-tree parse
    for (const r of rules) r.cssText;
    const p = doc.querySelector("#b > .c"); // dom-selector
    p.matches(".a, .zz");
    doc.querySelectorAll("div[data-x^='y'], p:not(.d)");
    win.getComputedStyle(p).getPropertyValue("color"); // css-color + cascade
    p.style.color = "rgb(10 20 30)";
    p.style.cssText;
    dom.window.close();
})();

// pre_globals.js force-loaded these onto globalThis only so undici's webidl module (imported
// by jsdom-bundle, evaluated above) had *some* Blob/File class to bind its type assertions to
// at cold-pass time, before any Window exists. Deleting them here -- after that binding has
// already happened, but before the snapshot is serialized -- keeps them out of bootstrap.js's
// `_preExistingGlobals` baseline, so each real Window's own spec-correct Blob/File (what
// jsdom's native FormData checks file-input entries against) still becomes the live global on
// every Runtime, exactly as it did before this cold-pass workaround existed.
delete globalThis.Blob;
delete globalThis.File;
