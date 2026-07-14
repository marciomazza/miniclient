# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

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
