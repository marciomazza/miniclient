// @ts-check
// Second pass of deno_core's two-pass snapshot: this runs against the cold snapshot, and the
// context it exercises is what ships. The snapshot is built with eager compilation, so this
// is not about compiling code; it builds the lazily initialised state (css-tree grammar,
// regexps, caches) that every Page would otherwise pay for first -- window construction, jsdom's HTML parser,
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
    const p = /** @type {Element} */ (doc.querySelector("#b > .c")); // dom-selector
    p.matches(".a, .zz");
    doc.querySelectorAll("div[data-x^='y'], p:not(.d)");
    win.getComputedStyle(p).getPropertyValue("color"); // css-color + cascade
    const styled = /** @type {HTMLElement} */ (p);
    styled.style.color = "rgb(10 20 30)";
    styled.style.cssText;
    dom.window.close();
})();

// A utility-framework-shaped sheet (the CSS a Tailwind-styled app ships): css-tree builds its per-property value grammar lazily, so
// what warms it is the *variety* of properties, value types and at-rules, not the rule count.
(() => {
    const decls = [
        "display:flex",
        "display:grid",
        "position:absolute",
        "inset:0",
        "top:calc(var(--spacing)*4)",
        "z-index:10",
        "float:left",
        "clear:both",
        "isolation:isolate",
        "overflow:hidden",
        "overflow-x:auto",
        "overscroll-behavior:contain",
        "visibility:hidden",
        "box-sizing:border-box",
        "width:calc(1/2*100%)",
        "min-width:0",
        "max-width:var(--container-4xl)",
        "height:100vh",
        "min-height:100dvh",
        "aspect-ratio:16/9",
        "margin:calc(var(--spacing)*2)",
        "margin-inline:auto",
        "padding:.5rem 1rem",
        "padding-block:max(1rem,env(safe-area-inset-top))",
        "gap:var(--spacing)",
        "grid-template-columns:repeat(3,minmax(0,1fr))",
        "grid-column:span 2/span 2",
        "flex:1 1 0%",
        "flex-direction:column",
        "flex-wrap:wrap",
        "align-items:center",
        "justify-content:space-between",
        "order:-1",
        "place-items:center",
        "font-family:var(--font-sans)",
        "font-size:var(--text-xs)",
        "font-weight:600",
        "font-style:italic",
        "line-height:calc(1.5/1)",
        "letter-spacing:-.025em",
        "text-align:center",
        "text-transform:uppercase",
        "text-decoration-line:underline",
        "text-overflow:ellipsis",
        "white-space:nowrap",
        "word-break:break-word",
        "vertical-align:middle",
        "list-style-type:none",
        "color:oklch(63.7% .237 25.331)",
        "color:color-mix(in oklab,currentcolor 50%,transparent)",
        "color:rgb(0 0 0/.5)",
        "color:hsl(210 40% 96%)",
        "color:#3b82f6",
        "color:transparent",
        "color:currentColor",
        "color:oklab(.5 .1 .1)",
        "background-color:rgb(255 255 255)",
        "background-image:linear-gradient(to right,var(--tw-gradient-from),var(--tw-gradient-to))",
        "background-position:center",
        "background-size:cover",
        "background-repeat:no-repeat",
        "border:1px solid #e5e7eb",
        "border-width:1px 0",
        "border-style:dashed",
        "border-color:oklch(87% .01 250)",
        "border-radius:calc(infinity*1px)",
        "border-collapse:collapse",
        "outline:2px solid transparent",
        "outline-offset:2px",
        "box-shadow:0 1px 3px 0 #0000001a,0 1px 2px -1px #0000001a",
        "opacity:.5",
        "mix-blend-mode:multiply",
        "filter:blur(4px) brightness(.5)",
        "backdrop-filter:blur(8px)",
        "transition:color .15s cubic-bezier(.4,0,.2,1),opacity .15s",
        "transition-property:all",
        "transition-duration:150ms",
        "animation:spin 1s linear infinite",
        "transform:translateX(-50%) rotate(45deg) scale(1.05)",
        "translate:var(--tw-translate-x) var(--tw-translate-y)",
        "transform-origin:top left",
        "cursor:pointer",
        "pointer-events:none",
        "user-select:none",
        "resize:vertical",
        "appearance:none",
        "content:var(--tw-content)",
        "content:''",
        "object-fit:cover",
        "scroll-behavior:smooth",
        "touch-action:manipulation",
        "will-change:transform",
        "accent-color:var(--color-black)",
        "caret-color:red",
        "fill:currentColor",
        "stroke-width:2",
        "table-layout:fixed",
        "text-wrap:balance",
        "-webkit-font-smoothing:antialiased",
        "--tw-ring-shadow:0 0 0 calc(1px + var(--tw-ring-offset-width)) var(--tw-ring-color)",
        "--tw-shadow:0 0 #0000",
        "--tw-space-y-reverse:0",
        "background:url(a.png) center/cover no-repeat",
        "background-position:left 10px top 20px",
        "background-position-x:50%",
        "background-image:url('b.png')",
        "background-image:radial-gradient(circle at top,red 10%,blue)",
        "background-image:linear-gradient(to bottom right,oklch(60% .2 250),#fff)",
        "border-top:1px solid red",
        "border-right:2px dashed",
        "border-bottom:thin",
        "border-left:0",
        "border-top-width:1px",
        "border-bottom-width:2px",
        "border-bottom-style:solid",
        "border-top-color:red",
        "border-right-color:#fff",
        "border-bottom-color:rgb(0 0 0)",
        "border-left-color:blue",
        "padding-top:1px",
        "padding-right:2px",
        "padding-bottom:3px",
        "padding-left:4px",
        "top:0",
        "right:0",
        "bottom:0",
        "left:0",
        "clip:rect(0,0,0,0)",
        "flex-grow:1",
        "flex-shrink:0",
        "flex-basis:auto",
        "flex:1",
        "flex:none",
        "font:italic 600 1rem/1.5 var(--font-sans)",
        "font:12px serif",
    ];
    const rules = [
        "@layer properties,theme,base,components,utilities;",
        "@layer theme{:root,:host{--font-sans:ui-sans-serif,system-ui,sans-serif;--spacing:.25rem;--text-xs:.75rem;--color-red-500:oklch(63.7% .237 25.331);--container-4xl:56rem}}",
        "@layer base{*,:after,:before,::backdrop{box-sizing:border-box;margin:0;padding:0;border:0 solid}html,:host{line-height:1.5;-webkit-text-size-adjust:100%}}",
        "@layer properties{@supports (((-webkit-hyphens:none)) and (not (margin-trim:inline))) or ((-moz-orient:inline) and (not (color:rgb(from red r g b)))){*,:before,:after,::backdrop{--tw-shadow:0 0 #0000;--tw-ring-color:initial}}}",
        "@keyframes spin{to{transform:rotate(360deg)}}",
        "@keyframes pulse{50%{opacity:.5}}",
        "@property --tw-translate-x{syntax:'*';inherits:false;initial-value:0}",
        "@font-face{font-family:x;src:url(x.woff2) format('woff2');font-display:swap}",
        "@scope (.card) to (.card-footer){p{color:red}}",
        "@container (min-width:20rem){.\\@sm\\:flex{display:flex}}",
        "@media (width>=40rem){.\\@md\\:flex{display:flex}}",
        "@media (400px<=width<=700px){.range\\:flex{display:flex}}",
        "@media print{.print\\:hidden{display:none}}",
        "@media (prefers-color-scheme:dark){.dark\\:bg-black{background-color:#000}}",
        "@media (prefers-reduced-motion:reduce){.motion-reduce\\:transition-none{transition-property:none}}",
    ];
    decls.forEach((d, i) => {
        rules.push(`.u${i}{${d}}`);
        if (i % 9 === 0) rules.push(`@media (min-width:${40 + i}rem){.md\\:u${i}{${d}}}`);
        if (i % 11 === 0) rules.push(`@supports (${d.split(";")[0]}){.s\\:u${i}{${d}}}`);
    });
    const dom = new JSDOM(
        "<style>" +
            rules.join("\n") +
            "</style>" +
            "<div class='card group'><input class='peer u3 u10' type=checkbox checked>" +
            "<button class='u1 u2 u6 u30 u31 u50' aria-expanded=true data-state=open disabled>b</button>" +
            "<p class='u40 u41 u44 u60' style='margin: 1px'>p</p></div>",
        { url: "about:blank" },
    );
    const win = dom.window;
    const doc = win.document;
    const walk = (/** @type {any} */ list) => {
        for (const r of list) {
            r.cssText;
            if (r.style) {
                for (let i = 0; i < r.style.length; i++) r.style.getPropertyValue(r.style[i]);
            }
            if (r.cssRules) walk(r.cssRules);
        }
    };
    walk(doc.styleSheets[0].cssRules);
    for (const el of doc.querySelectorAll("*")) {
        const cs = win.getComputedStyle(el);
        cs.getPropertyValue("color");
        cs.getPropertyValue("display");
    }
    dom.window.close();
})();

globalThis.__clearDomBindings();
delete (/** @type {any} */ (globalThis).__clearDomBindings);
