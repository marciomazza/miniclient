# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

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
