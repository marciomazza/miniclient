---
status: accepted
---

# No single synchronous native `Page` core

A pyo3 method must decide, in its own signature (`fn` vs `async fn`), whether to block or
await — but whether `eval()` needs to await (the script returned a Promise) is only known
after running the script. Making it unconditionally blocking cannot service `fetch()`
without deadlocking (the Python-side fetch callback needs an asyncio loop pumping
concurrently, which a blocked thread can't do), and every workaround explored that keeps
`eval()` blocking (a dedicated loop thread, routing `fetch()` through the synchronous
bridge, an ephemeral dispatch channel) loses `fetch()` cancellation, which today depends on
a real, cancellable asyncio Task.

Decided to keep the DOM-ergonomic methods (which equally need to wait for the page to
settle, not just `eval()`) on an async-only native core (`browser::Page`/`browser::Element`,
Rust, pyo3-agnostic), exposed through a single always-async `eval()`. The existing
synchronous Python facade (`Page`/`Element`, running its own event loop over the async
core) stays as the only synchronous entry point, instead of building a true single-threaded
synchronous native core.
