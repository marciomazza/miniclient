# Architecture

## Runtime stack

- **jsrun** (V8 via deno_core + PyO3 bindings) is the JavaScript runtime. It is NOT Node.js and NOT QuickJS.
- **happy-dom** runs INSIDE jsrun, loaded with custom module polyfills for Node modules (`node:buffer`, `node:stream`, `node:crypto`, etc.).
- **htmx** runs inside the same jsrun context, initialized after happy-dom's `Window` is set up. It uses the polyfilled `fetch` and timers. `fetch` is done via `httpx`
- The Python `Browser` class in `src/htmxclient/browser.py` wraps a jsrun `Runtime`. It has NO relation to happy-dom's `Browser` class.

## DOM interaction

All DOM interaction (click, submit, dispatchEvent, query selectors, etc.) is done by evaluating JavaScript inside the jsrun runtime via `runtime.eval()` / `runtime.eval_async()`. There is no direct Python API to happy-dom or htmx objects — they live inside the V8 isolate.

When adding DOM interaction methods, implement them in Python using `runtime.eval()` / `runtime.eval_async()` to execute JS against the happy-dom `document` already initialized in `bootstrap.js`.
