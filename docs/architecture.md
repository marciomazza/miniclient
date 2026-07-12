---
icon: lucide/layers
---

# Architecture

## Comparison to other tools

Browser automation tools — [Playwright](https://playwright.dev/), [Selenium](https://www.selenium.dev/), [Puppeteer](https://pptr.dev/) — drive a real browser (Chromium, Firefox, WebKit) over a devtools/wire protocol: real rendering, real JS engine, maximum fidelity, but you pay for browser process startup, IPC round-trips, and often GPU/rendering work your test might not need. This project instead runs actual JavaScript code against a real DOM implementation ([happy-dom](https://happy-dom.dev/)) inside an embedded V8 isolate, in-process — no browser process, no protocol, no rendering. That makes the common case (assert a request/DOM-swap/event loop happened correctly) dramatically faster, at the cost of not being a full browser: no layout/paint, no CSS cascade beyond what happy-dom implements, and behavior can diverge anywhere happy-dom or this project's polyfills differ from a real engine (see *Known limitations*).

## How this was made

### Test Methodology

Correctness is checked at three layers:

- **happy-dom integration** — regression tests cover local patches applied to happy-dom and the Node polyfills it depends on.
- **htmx compat** — we test specifically for htmx's correct functioning in this environment,
  and further guarantee the correctness of the setup as a whole.
- **Crosscheck against a real browser** — stateful fuzzing exercises randomly generated pages and interactions, checking that DOM and request/response behave the same as in an actual browser.

#### htmx compat

htmx's own original test suite runs directly inside this runtime: one pytest case per Mocha JS file, with a small explicit skip-list for things that can't be true headlessly (e.g. scroll position). This anchors behavior to the upstream's own definition of correctness.

#### Hypothesis crosscheck against a real browser

We use [Hypothesis](https://hypothesis.readthedocs.io) for property based testing.

Hypothesis drives stateful fuzzing that goes further than any hand-written tests. It generates random HTML — form controls, htmx-attributed elements, whole "rich pages" mixing both, wired to a test app — and draws random sequences of interactions (fill, click, dispatch_event) against those generated pages. After every step, a `CrossCheck` harness replays the same interaction against both this project's `Browser` (V8/happy-dom) and a real Chromium page via Playwright, asserting identical DOM snapshots and HTTP request/response parameters. Any divergence, however deep in a randomly generated interaction sequence, fails the test and Hypothesis shrinks it to a minimal reproduction. This process catches subtle DOM/protocol fidelity gaps between happy-dom-in-V8 and a real browser that no simple test suite would think to check.

## Known limitations

**No real Node.js.** The JavaScript code runs inside a V8 Isolate, not Node.
There's no node_modules resolution, no require(), and no *package.json* lookup — only ESM import against a closed allowlist of pre-vendored packages; anything else fails to resolve. In practice this means arbitrary npm packages that depend on the Node APIs won't run as-is.

But notice that *happy-dom* itself is a Node package and depends on many parts of the Node API.
It works because [*jsrun* module system](https://imfing.github.io/jsrun/guides/modules) allows us to map `node:*` specifiers to *polyfills* (that we made) covering the Node API surface that it touches.
Follow the same approach — a resolver plus hand-written polyfills — for any other Node-dependent package you need to run.

A [Vite](https://vite.dev/) production bundle sidesteps this almost entirely: bundling resolves the whole import graph at build time, so nothing needs runtime `require()` or bare-specifier resolution — a standard browser-targeted build just works. The exception is Node-targeted output (SSR, `target: 'node'`) that still calls real `fs`/`child_process`/`os` at runtime — bundling flattens imports, not the runtime APIs, so that code hits the same missing-Node-API wall as unbundled code.

**One window per runtime.** A jsrun `Runtime` wraps a single `deno_core::JsRuntime` with one V8 `globalThis`, so there's no true per-window isolation — this only matters if the project ever needs to support multiple simultaneous `Window` instances inside one `Runtime`, which isn't possible today. Globals persist across navigations rather than resetting per-page as in a real browser. This is usually acceptable for a test scenario but is important to keep in mind.
