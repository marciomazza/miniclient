# IMPORTANT - strictly follow this

**Detect if this is a jujutsu repo and if it is:**
  - NEVER use any git command.
  - At the start of each session, before any write: if not in an empty change, run `jj new`
  - To commit: `jj describe -m "..."` then `jj new`

**Commit messages and code in English**

## graphify

This project may have a local knowledge graph at graphify-out/ (god nodes, community structure, cross-file relationships).

Rules:
- ALWAYS: IF graphify-out/GRAPH_REPORT.md exists, read it before searching raw files or answering codebase questions — it's the primary map of the codebase.
- IF graphify-out/wiki/index.md exists, navigate it instead of reading raw files
- For cross-module "how does X relate to Y" questions, prefer `graphify query "<question>"`, `graphify path "<A>" "<B>"`, or `graphify explain "<concept>"` over grep — these traverse the graph's EXTRACTED + INFERRED edges instead of scanning files
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
- If `graphify-out-deps/graph.json` exists (not guaranteed in every checkout), it's a separate graph of external deps (happy-dom, jsrun, vendored htmx tests). For questions about their internals, add `--graph graphify-out-deps/graph.json` to `query`/`explain`/`path`. Report: `graphify-out-deps/GRAPH_REPORT.md`.

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

When you make a variable that contains a string with javascript code, name it either `js` or with the suffix `_js`.
