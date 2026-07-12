---
icon: lucide/layers
---

# Architecture

## Runtime stack

- **[jsrun](https://imfing.github.io/jsrun)** (V8 via deno_core + PyO3 bindings) is the JavaScript runtime. It is NOT Node.js and NOT QuickJS.
- **[happy-dom](https://github.com/capricorn86/happy-dom)** runs inside jsrun, loaded with custom module polyfills for Node modules (`node:buffer`, `node:stream`, `node:crypto`, `node:vm`, etc.) in `src/miniclient/js/polyfills/`.
- **[htmx](https://htmx.org)** runs in the same jsrun context, loaded like a real page would load it — via `<script src="...">`, resolved through happy-dom's `virtualServers` mechanism (mapped from the `mounts` param on `Browser.create` / `virtual_servers` param on `build_runtime`). It is not baked into the runtime snapshot.
- HTTP is done with **[httpx2](https://httpx2.pydantic.dev/)**.
- The Python `Browser` class in `src/miniclient/browser.py` wraps a jsrun `Runtime`. It has no relation to happy-dom's own `Browser` class.

## Limitations and consequences

1. **No true per-window isolation.** A single jsrun `Runtime` has one real V8 `globalThis`. If two `Window`/`Browser` instances ever shared one `Runtime`, their script globals would collide instead of staying scoped to their own window.
2. **Globals persist across navigations.** `htmx` (or anything else a script declares) is never reset just because the "page" navigated, unlike a real browser's fresh-global-per-navigation model. Only closing/recreating the `Runtime` clears it.
3. **`this` binding differs from a real `<script>` tag** for scripts run through the classic-script execution path.
4. **`"use strict"` scripts** don't get the same global-leak workaround — strict-mode code keeps `var`/`function` scoped to the eval call itself.
5. **No engine-level fix is available today.** True per-context globals (separate V8 contexts per window) would require a jsrun-level feature; today's `Runtime` wraps a single `deno_core::JsRuntime` with one main context.

These tradeoffs are acceptable for the current test/single-window usage pattern but should be revisited if this project ever needs multiple concurrently-live windows/browsers sharing one `Runtime`.
