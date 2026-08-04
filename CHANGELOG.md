# Changelog

The main changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed

- `click()`/`trigger()` events weren't `cancelable`, so a page's own `preventDefault()` (e.g.
  htmx) couldn't stop happy-dom's native default action from also firing and replacing the window.

## [0.1.1]

### Fixed

- Fixed bundle staleness check (for dev) breaking the wheel.
- Fixed an intermittent segfault under heavy `Runtime()` construction/close churn
  (e.g. the htmx test suite), caused by a race in jsrun's isolate teardown.
  Our solution is a workaround and the real fix belongs upstream in jsrun.

## [0.1.0]

### Added

- `Page`/`AsyncPage.url` property for `location.href`.

### Changed

- Renamed `Browser`/`AsyncBrowser` to `Page`/`AsyncPage` (module `miniclient.browser` is now
  `miniclient.page`), matching happy-dom's and playwright vocabulary.
- POST form submissions with no explicit `enctype` are now sent as
  `application/x-www-form-urlencoded`, matching real browsers, instead of always as
  `multipart/form-data`.
- `goto()`/`load()` now drive real happy-dom navigation instead of simulating it via
  `document.write()`. Consequence: htmx (and any other page script) is now page-scoped like a
  real browser, not persistent across navigations — this fixes event listeners and
  `hx-trigger="every ...s"` polling timers accumulating across repeated `goto()`/`load()` calls
  within a test.
- `Page` now runs on a same-thread event loop instead of a dedicated background thread:
  `eval()` \~7x faster, `find()` \~4x faster, `click()` \~15-20% faster.
- `click()`/`trigger()`/`fill()`/`requestSubmit()` now wait for the page to settle (any pending
  timers/fetches) instead of specifically waiting for htmx to settle, so they behave the same on
  htmx pages, other-framework pages, and plain pages.
- Upgraded happy-dom to 20.11.1.
- Performance: happy-dom is now baked into the V8 snapshot instead of imported per `Runtime`,
  dropping `open_runtime()` from \~110ms to \~16-18ms.

### Fixed

- `AsyncPage.load()` now waits for a synthesized `DOMContentLoaded` event (happy-dom never fires
  one on its own), so app code gated behind that event now runs on initial load.
- `AsyncPage.__enter__` now fails fast if used before the page is built.
- Fixed `Element.find()`/`find_all()`/`.parent` being statically typed as returning `AsyncElement`
  even for sync `Element` instances.
- Fixed `attachInternals` errors being silently swallowed instead of propagating.
- Fixed module scripts (`<script type="module" src>`) never executing.
- Polyfilled `globalThis.crypto`, fixing app code that calls `crypto.getRandomValues()` during
  module evaluation (e.g. TinyMCE).
- Fixed several happy-dom bugs affecting DOM/htmx behavior: `Event.timeStamp` reading a
  user-mocked `performance.now`, form `reset()` ignoring `<select multiple>`, `location.hash`
  clobbering `history.state`, missing `Document.parseHTMLUnsafe`, a colon-attribute/plain-name
  sibling corruption bug, missing `:required`/`:invalid`/`:valid` pseudo-classes and `:disabled`
  fieldset propagation, form/select proxy identity loss when moved in the DOM, form-associated
  custom elements missing from `HTMLFormElement.elements`, and a `MutationObserver` GC bug
  affecting libraries like Alpine.js.
- Fixed `ReadableStream` backpressure and streaming-decode bugs affecting SSE and multipart
  streaming response bodies.
- Fixed `AsyncPage.aclose()` skipping `close()` for pages built without an injected `runtime=`
  (e.g. via `mounts=`), which silently bypassed any `close()` override/instrumentation and left
  `close()` alone unable to tear down the pooled httpx client for such pages.

## [0.0.10]

- Added `Element`/`AsyncElement.parent` property.
- Fixed the `:checked` pseudo-class to also match a selected `<option>`, not just
  checked `<input>` elements, per spec.
- `Element`/`AsyncElement.fill()` now dispatches a `change` event and waits for htmx to
  settle, matching `click()`/`trigger()`. Previously it only set `.value` directly, so
  htmx's default `change` trigger (and any `hx-trigger` listening for it) never fired.
- Fixed `HTMLSelectElement.value` not invalidating happy-dom's `:checked` query cache,
  causing stale results for `:checked` selectors evaluated before a selection change.

## [0.0.9]

- Fixed a regression crash during script evaluation caused by missing
  Symbol-keyed `Window`/`BrowserWindow` prototype methods on `globalThis`

## [0.0.8]

- `Browser`/`AsyncBrowser` `find()`/`find_all()` now accept an optional `text` argument to filter
  matches by contained text (a substring match against `textContent`).
- `Element`/`AsyncElement` now expose `find()` and `find_all()`, scoped to the
  element instead of the whole document.
- The bootstrap global wiring now follows the `@happy-dom/global-registrator`
  pattern, making `window === globalThis` and improving compatibility with
  browser globals and `fetch` mocks.

## [0.0.7]

- `Browser`/`AsyncBrowser` DOM actions that trigger an htmx request (`click`, `submit`, ...) now
  wait for scripts and resources loaded during the swap to finish, not just the request itself.
- `Browser`/`AsyncBrowser` now reuse a single `httpx.AsyncClient` for every request (async and
  sync fetch alike) for the life of the browser, instead of opening a new client per call. Fixes
  an `AttributeError` from sync fetch when a custom async-only `httpx_transport` (like
  `WSGITransport`) is used, and unifies cookies/connection pooling/redirects across sync and
  async requests.

## [0.0.6]

- `Browser/AsyncBrowser.goto()` now follows redirects.
- Removed the `url` parameter from `Browser`/`AsyncBrowser` constructors; use `goto()` to navigate.

## [0.0.5]

- Cookies are now sent/stored through `fetch()`, using happy-dom's own cookie jar
  (so `HttpOnly` session cookies, e.g. Django's `sessionid`, work correctly).
- `Element.html`, `.innerHTML` and `.text` are now properties instead of methods.

## [0.0.4]

Initial release.

- A minimal python http client for testing that runs JavaScript
  inside a V8 Isolate with happy-dom, especially tested for `htmx`.
- DOM interaction (load, query, click, submit, dispatch events) via JS evaluated
  in the runtime.
- happy-dom patched/polyfilled to fix divergences from real browser behavior.
