# IMPORTANT - strictly follow this

**Detect if this is a jujutsu repo and if it is:**
  - NEVER use any git command.
  - At the start of each session, before any write: if not in an empty change, run `jj new`
  - To commit: `jj describe -m "..."` then `jj new`

**Commit messages and code in English**

# Architecture

## Runtime stack

- **jsrun** (V8 via deno_core + PyO3 bindings) is the JavaScript runtime. It is NOT Node.js and NOT QuickJS.
- **happy-dom** runs INSIDE jsrun, loaded with custom module polyfills for Node modules (`node:buffer`, `node:stream`, `node:crypto`, etc.).
- **htmx** runs inside the same jsrun context, initialized after happy-dom's `Window` is set up. It uses the polyfilled `fetch` and timers. `fetch` is done via `httpx`
- The Python `Browser` class in `src/htmxclient/browser.py` wraps a jsrun `Runtime`. It has NO relation to happy-dom's `Browser` class.

More details about htmx:
@vendor/htmx/src/skills/htmx-guidance.md

## DOM interaction

All DOM interaction (click, submit, dispatchEvent, query selectors, etc.) is done by evaluating JavaScript inside the jsrun runtime via `runtime.eval()` / `runtime.eval_async()`. There is no direct Python API to happy-dom or htmx objects — they live inside the V8 isolate.

When adding DOM interaction methods, implement them in Python using `runtime.eval()` / `runtime.eval_async()` to execute JS against the happy-dom `document` already initialized in `bootstrap.js`.

In js code strings (e.g. in evals) try to avoid using escaped quotes:
use triple quotes around instead. E.g.:
```
  runtime.eval(f"""\
    const html = "<div><p id='x'></p></div>"
  """)
```
