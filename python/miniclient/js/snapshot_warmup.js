// Second pass of deno_core's two-pass snapshot: this runs against the cold snapshot, and the
// context it exercises is what ships, so whatever it compiles is already compiled at Page
// load. It warms what every Page pays for first -- window construction, happy-dom's HTML
// parser, DOM traversal, xpath and FormData. It ships in production, so it must never touch
// anything from tests/.
//
// What it leaves out is dictated by V8's snapshot serializer, which segfaults on a warmed
// context that ever constructed a `WeakRef` -- that rules out happy-dom's live collections
// (getElementsByTagName) and its CSS selector engine (querySelector/querySelectorAll), and
// reading `document.body` trips the serializer too. htmx is out for a different reason: it is
// not in the snapshot at all, a page loads it from a <script> tag. The hx-* attributes below
// still warm the parser path htmx's own scan walks.
//
// The Rust test `production_scripts_and_warmup_produce_a_bootable_snapshot` is the guard: it
// fails if anything added here stops the warmed context from serializing.
(() => {
    const page = new globalThis.__happyDomBundle.Browser().newPage();
    const root = page.mainFrame.window.document.createElement("div");
    root.innerHTML =
        '<button hx-get="/warm" hx-target="#warm">go</button>' +
        '<form><input name="a" value="1"></form>';
    root.firstElementChild.getAttribute("hx-get");

    new globalThis.XPathEvaluator().createExpression("//button").evaluate(root).iterateNext();

    const form = new globalThis.FormData();
    form.append("a", "1");
    [...form.entries()];

    page.close();
})();
