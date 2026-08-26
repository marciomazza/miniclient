---
icon: lucide/zap
---

# miniclient

A minimal **python http client that runs real JS/DOM**, without a browser.<br>
Meant to be used as a lightweight test client to simulate browser interactions.<br>
Embeds a V8 Runtime, DOM, and is designed to run [htmx](https://htmx.org) especially well.

_This project is under active development.
The API is experimental and might change.
We've got a lot of tests but things might break._

## Main Components

- **[httpx2](https://httpx2.pydantic.dev/)** for a fully featured HTTP client, with special support
  for testing WSGI/ASGI apps (Django, Flask, FastAPI).
- **[V8](https://v8.dev/)** via **[deno_core](https://github.com/denoland/deno)** for the JavaScript runtime. No Node.js. No browser.
- **[happy-dom](https://github.com/capricorn86/happy-dom)** for a fast DOM implementation in pure JavaScript.

## htmx

- **[htmx](https://htmx.org)** integration is thoroughly tested.
  The complete htmx test suite passes, including extensions, and most of the design was done to support it.

  _We currently support only htmx version 4_

## Why?

Testing against a real browser feels too slow. And mostly unnecessary.
