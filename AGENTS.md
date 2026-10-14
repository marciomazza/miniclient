# IMPORTANT - strictly follow this

## Agent skills

### Coding standards

Commit messages and code in English; comment rules; naming
and quoting conventions for JS eval strings. See [docs/coding-standards.md](docs/coding-standards.md).
Prefer to write tests in rust as much as possible. Write python tests only for what touches the python layer.
Run Rust tests with `cargo nextest run`, including targeted test runs.

### Issue tracker

Issues, specs, and wayfinder maps live as markdown under `.scratch/<feature-slug>/`. See
`docs/agents/issue-tracker.md`.

### Triage labels

Five canonical roles (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`,
`wontfix`), used as-is. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.

## graphify

This project may have a local knowledge graph at graphify-out/ (god nodes, community structure, cross-file relationships).

Rules:

- ALWAYS: IF graphify-out/GRAPH_REPORT.md exists, read it before searching raw files or answering codebase questions — it's the primary map of the codebase.
- IF graphify-out/wiki/index.md exists, navigate it instead of reading raw files
- For cross-module "how does X relate to Y" questions, prefer `graphify query "<question>"`, `graphify path "<A>" "<B>"`, or `graphify explain "<concept>"` over grep — these traverse the graph's EXTRACTED + INFERRED edges instead of scanning files
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
- If `graphify-out-deps/graph.json` exists (not guaranteed in every checkout), it's a separate graph of external vendored deps.
  For questions about their internals, add `--graph graphify-out-deps/graph.json` to `query`/`explain`/`path`. Report: `graphify-out-deps/GRAPH_REPORT.md`.

## vendor/htmx

Vendored upstream code, untracked. Treat as read-only:

- Never edit it to fix bugs or add features — changes are lost on vendor update.
- Debugging edits (logs, instrumentation) are OK but must be reverted before finishing.
- Found a real bug in it? Report it, don't patch it.

# Architecture

This project is an integration-testing tool that simulates a browser environment with
JavaScript. htmx (vendored) is one of the workloads validated inside that simulated
environment.

## Design principle: mini is a shared test dependency

mini is a library other projects' test suites run against — htmx's vendored suite is one
consumer, not the spec. A polyfill gap here silently changes behavior for every downstream
app too. Prefer spec-correct behavior over the minimum needed to turn mini's own suite green.

## Runtime stack

- A JavaScript runtime provide by deno_core. It is NOT Node.js and NOT QuickJS.
- **jsdom** runs INSIDE that runtime, loaded with custom module polyfills for Node modules (`node:buffer`, `node:stream`, `node:crypto`, etc.).
- **htmx** runs inside the same context, initialized after jsdom's `Window` is set up. It uses the polyfilled `fetch` and timers. `fetch` is done via `httpx`
- The Python `Page` class in `python/miniclient/browser.py` wraps a `Page` pyo3 class (imported there as `RuntimePage` to avoid colliding with the facade's own name) — one window/document/frame, no tabs, mirroring what Playwright's own `Browser.newPage()` returns.

More details about htmx:
@vendor/htmx/src/skills/htmx-guidance.md

## DOM interaction

All DOM interaction (click, submit, dispatchEvent, query selectors, etc.) is done by evaluating JavaScript inside the runtime via `runtime.eval()` / `runtime.eval_async()`. There is no direct Python API to jsdom or htmx objects — they live inside the V8 isolate.

When adding DOM interaction methods, implement them in Python using `runtime.eval()` / `runtime.eval_async()` to execute JS against the jsdom `document` already initialized in `bootstrap.js`.

For naming and quoting conventions in these JS eval strings, see
[docs/coding-standards.md](docs/coding-standards.md).

# Build and release

See [docs/release.md](docs/release.md) — read it before building, bumping the version,
tagging, or releasing.

- The wheel ships only vendored licences, not vendored source. This is fine because every
  vendored dep is permissive (MIT/BSD/ISC/Apache) and the source is already inlined in the
  bundle and snapshot. When adding a new vendored dep, confirm its licence is MIT/BSD/ISC/
  Apache before treating it the same way — a copyleft licence (GPL/LGPL/EPL) would require
  shipping the source.
- Unmodified MPL-2.0 Cargo crates (stylo, selectors, cssparser, ...) are allowed: the wheel
  carries `python/miniclient/MPL_NOTICE.md`, generated from `Cargo.lock` by `just notice` and
  checked by `tests/third_party_notice.rs`. Never patch or vendor an MPL file. A dual-licensed
  crate offering MPL fails that test until it is added to `ELECTED`.
