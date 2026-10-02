# mini

A library that drives one simulated browser window per `Page`, running real JS/DOM (jsdom)
without a real browser, for integration testing. htmx is one workload validated inside that
environment, not a project dependency.

## Language

**Page**:
The user-facing handle to one browser window/document/frame — no tabs, mirrors
Playwright's `Browser.newPage()`.
_Avoid_: Browser (retired name from an earlier design; caused confusion with the lower,
eval-only primitive underneath `Page`).

**Element**:
A reference to a single DOM node, usable independently of the `Page` that produced it —
stays alive and tears down silently even if its `Page` has already closed.
_Avoid_: node, handle (a handle is the internal id an `Element` carries, not the object
itself).

**FormElement**:
An `Element` that is specifically a `<form>`, adding `requestSubmit()`. Kept as its own
type rather than folded into `Element`.

**eval**:
Runs arbitrary JavaScript in the page and returns its result, automatically awaiting a
returned Promise. The caller never chooses between a sync and an async variant of eval.

**settle**:
The point where the page's real event loop has gone idle — not merely that a returned
Promise resolved, since a handler can fire further work (e.g. a fetch) that outlives its
own promise chain. Ergonomic methods that interact with the page wait for settle, not
just for their own promise.
