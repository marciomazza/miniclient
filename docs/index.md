---
icon: lucide/zap
---

# miniclient

A minimal **python http client that runs JavaScript**, without a browser.<br>
Meant to be used as a lightweight test client to simulate browser interactions.<br>
Embeds a V8 Runtime, DOM, and is designed to run [htmx](https://htmx.org) especially well.

*This project is under active development.
The API is experimental and might change.
We've got a lot of tests but things might break.*

## Main Components

- **[httpx2](https://httpx2.pydantic.dev/)** for a fully featured HTTP client, with special support
  for testing WSGI/ASGI apps (Django, Flask, FastAPI).
- **[jsrun](https://imfing.github.io/jsrun)** (V8 via deno_core + PyO3 bindings) for the JavaScript runtime.
No Node.js.
- **[happy-dom](https://github.com/capricorn86/happy-dom)** for a fast DOM implementation in pure JavaScript.

## htmx

- **[htmx](https://htmx.org)** integration is thoroughly tested.
  The complete core htmx test suite passes and most of the design was done to support it.

  *We currently support only htmx version 4*

## Why?

Testing against a real browser feels too slow. And mostly unnecessary.
