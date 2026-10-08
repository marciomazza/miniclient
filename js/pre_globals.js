(function () {
    // Real, circular-safe console (replaces deno_core's built-in one, which throws on any
    // circular arg -- htmx passes those). 00_url.js is force-loaded too: 01_console.js
    // lazy-loads it on first use, and only scripts touched here (the snapshot's cold pass)
    // survive into the snapshot's residual lazy table.
    //
    // URL/URLSearchParams — deno_web's WHATWG impl (Servo `url` crate). Assigned here in the
    // cold pass so the bundle's `require('node:url')` imports resolve to the same globals;
    // node-url.js re-exports them for the `url` bundle alias, which undici's
    // socks5-proxy-agent pulls in. searchParams mutations propagate back to href, and
    // URLSearchParams accepts an iterable (incl. our FormData) as init.
    {
        const url = Deno.core.loadExtScript("ext:deno_web/00_url.js");
        const { Console } = Deno.core.loadExtScript("ext:deno_web/01_console.js");
        globalThis.console = new Console(Deno.core.print);

        Object.assign(globalThis, { URL: url.URL, URLSearchParams: url.URLSearchParams });
    }

    // TextEncoder/TextDecoder — deno_web's, backed by encoding_rs ops. Force-loaded here
    // (cold snapshot pass) so the classes land on globalThis for the bundle's import
    // resolution. webidl must be touched too: only ext scripts loaded in the cold pass
    // survive the snapshot's residual lazy table.
    {
        Deno.core.loadExtScript("ext:deno_webidl/00_webidl.js");
        const te = Deno.core.loadExtScript("ext:deno_web/08_text_encoding.js");
        Object.assign(globalThis, {
            TextEncoder: te.TextEncoder,
            TextDecoder: te.TextDecoder,
            TextEncoderStream: te.TextEncoderStream,
            TextDecoderStream: te.TextDecoderStream,
        });
    }

    globalThis.process = {
        platform: "linux",
        arch: "x64",
        env: {},
        version: "",
        versions: {},
        argv: ["node"],
        // jsdom schedules internal microtasks (e.g. event dispatch) through this; a plain
        // microtask queue is a faithful enough stand-in since nothing here needs macrotask
        // ordering relative to I/O. `queueMicrotask` itself is out: deno_core's own
        // implementation calls back into a bootstrap-time internal helper that never runs
        // during a cold snapshot pass, so it throws here -- `Promise.resolve().then()` reaches
        // the same native V8 microtask queue without going through that helper.
        nextTick: (fn, ...args) => Promise.resolve().then(() => fn(...args)),
    };

    // performance — deno_web's real implementation: monotonic now() (op_now), working
    // marks/measures/observers. Force-loaded here (cold snapshot pass) so the bundle's
    // `require('node:perf_hooks')` resolves to the same singleton and classes; node-perf-hooks.js
    // re-exports PerformanceObserver/PerformanceEntry from the globals set below.
    {
        // 15_performance lazy-loads structured_clone for mark/measure detail; force it into
        // the snapshot's residual table here since nothing else touches it in the cold pass.
        globalThis.structuredClone = Deno.core.loadExtScript(
            "ext:deno_web/02_structured_clone.js",
        ).structuredClone;
        const perf = Deno.core.loadExtScript("ext:deno_web/15_performance.js");
        perf.setTimeOrigin();
        Object.assign(globalThis, {
            performance: perf.performance,
            Performance: perf.Performance,
            PerformanceEntry: perf.PerformanceEntry,
            PerformanceMark: perf.PerformanceMark,
            PerformanceMeasure: perf.PerformanceMeasure,
            PerformanceObserver: perf.PerformanceObserver,
            PerformanceObserverEntryList: perf.PerformanceObserverEntryList,
        });
    }

    // crypto — CSPRNG backed by native ops (getrandom + the uuid crate). Required for
    // `require('node:crypto')` resolution: jsdom's Crypto-impl and undici's WebSocket /
    // formdata paths pull randomUUID / randomFillSync / randomInt from there. The polyfill
    // forwards this global as `webcrypto`. Only getRandomValues + randomUUID are used
    // anywhere; no crypto.subtle.
    globalThis.crypto ??= {
        getRandomValues(arr) {
            const bytes = new Uint8Array(Deno.core.ops.op_crypto_random_bytes(arr.byteLength));
            new Uint8Array(arr.buffer, arr.byteOffset, arr.byteLength).set(bytes);
            return arr;
        },
        randomUUID() {
            return Deno.core.ops.op_crypto_random_uuid();
        },
    };

    // SharedArrayBuffer não existe neste runtime: o deno_core monta o contexto V8 de forma
    // minimalista e nunca passa pelo bootstrap que instalaria esse construtor condicional
    // (WeakRef e Atomics continuam presentes, só este falta) -- precisa existir antes do
    // jsdom ser importado: generated/idl/utils.js e webidl-conversions leem os descriptors
    // "byteLength"/"growable" de `SharedArrayBuffer.prototype` no topo do módulo, mesmo que
    // nada que o mini exercite construa um de verdade.
    globalThis.SharedArrayBuffer ??= class SharedArrayBuffer {
        #buf;
        constructor(length = 0) {
            this.#buf = new ArrayBuffer(length);
        }
        get byteLength() {
            return this.#buf.byteLength;
        }
        get growable() {
            return false;
        }
        slice(...args) {
            return this.#buf.slice(...args);
        }
    };

    // jsdom calls bare `setTimeout(...)` from a handful of impl files (Storage events,
    // HTMLInputElement/HTMLTextAreaElement select events, AbortSignal, SessionHistory).
    // Those resolve through globalThis.setTimeout; if it were deno_core's no-op stub the
    // queued work would never run. This is the single source of truth for the whole
    // bundle: deno_web's own timer implementation, force-loaded like atob/btoa below. Its
    // ref-counting is left untouched: run_event_loop only drains currently-resolvable work,
    // never blocks on a future-scheduled tick regardless of ref state, so a forever htmx
    // poller (hx-trigger="every Ns") already can't pin click()/submit()'s post-eval drain --
    // confirmed by test_click_not_blocked_by_unrelated_poller.
    // Picked onto globalThis by name, not kept behind a `timers.setTimeout(...)` object
    // (02_timers.js's own `checkThis` rejects any `this` other than undefined/globalThis,
    // which a method call through an intermediate object would bind to that object instead)
    // and not spread wholesale (the module also exports non-standard `defer`/`refTimer`/
    // `unrefTimer`, which have no business being page-visible globals).
    const timers = Deno.core.loadExtScript("ext:deno_web/02_timers.js");
    globalThis.setTimeout = timers.setTimeout;
    globalThis.setInterval = timers.setInterval;
    globalThis.clearTimeout = timers.clearTimeout;
    globalThis.clearInterval = timers.clearInterval;
    globalThis.setImmediate ??= (fn, ...a) => setTimeout(fn, 0, ...a);
    globalThis.clearImmediate ??= () => {};

    // Base64 — not built into V8; needed by entities and node-buffer.js polyfill.
    // Force-loaded here (like 00_url.js) so it survives into the snapshot.
    {
        const { atob, btoa } = Deno.core.loadExtScript("ext:deno_web/05_base64.js");
        globalThis.atob = atob;
        globalThis.btoa = btoa;
    }

    // WHATWG Streams — deno_web's spec reference implementation: real backpressure
    // (desiredSize) and correct EOF (a pending read() waits for a later enqueue()/close()
    // rather than resolving done when the queue drains). Force-loaded here (cold snapshot
    // pass) so the bundle's `require('node:stream/web')` imports resolve to the same
    // global classes -- undici's Request body does an `instanceof ReadableStream` check
    // that needs the exact identity.
    // 06_streams.js lazy-loads 03_abort_signal.js + 00_infra.js, pulled in transitively here.
    {
        const s = Deno.core.loadExtScript("ext:deno_web/06_streams.js");
        Object.assign(globalThis, {
            ReadableStream: s.ReadableStream,
            WritableStream: s.WritableStream,
            TransformStream: s.TransformStream,
            ReadableStreamDefaultReader: s.ReadableStreamDefaultReader,
            ReadableStreamBYOBReader: s.ReadableStreamBYOBReader,
            ReadableStreamBYOBRequest: s.ReadableStreamBYOBRequest,
            ReadableStreamDefaultController: s.ReadableStreamDefaultController,
            ReadableByteStreamController: s.ReadableByteStreamController,
            WritableStreamDefaultWriter: s.WritableStreamDefaultWriter,
            WritableStreamDefaultController: s.WritableStreamDefaultController,
            TransformStreamDefaultController: s.TransformStreamDefaultController,
            ByteLengthQueuingStrategy: s.ByteLengthQueuingStrategy,
            CountQueuingStrategy: s.CountQueuingStrategy,
        });
    }

    // Blob/File/AbortController/AbortSignal — deno_web's real implementations, force-loaded
    // (like streams above) so the jsdom bundle's cold-pass import of undici's Request/Response
    // (jsdom-entry.js) sees these globals: its webidl layer references them at module scope
    // (e.g. `webidl.is.Blob = webidl.util.MakeTypeAssertion(Blob)`), before any Window exists.
    // Blob/File are deleted again at the end of snapshot_warmup.js: each real Window has its
    // own spec-correct versions (what jsdom's native FormData checks file-input entries
    // against), and must still be free to become the live global once one exists --
    // AbortController/AbortSignal stay, because jsdom's own resource-loader interceptor
    // (request-interceptor.js) does `new AbortController()` + `new Request(url, { signal })`
    // internally, and that `Request` is undici's (also bound to this same deno_web
    // AbortSignal at cold-pass, see below) -- swapping AbortController/AbortSignal for a
    // per-Window version here would desync that pairing and break every subresource fetch,
    // not just the one thing it was meant to fix (jsdom's `EventTarget#addEventListener`
    // brand-checking an `{ signal }` option against its own internal AbortSignal class,
    // rejecting this deno_web one -- patched instead in jsdom itself, see
    // patches/jsdom+30.0.1+001+initial.patch).
    {
        const f = Deno.core.loadExtScript("ext:deno_web/09_file.js");
        const a = Deno.core.loadExtScript("ext:deno_web/03_abort_signal.js");
        Object.assign(globalThis, {
            Blob: f.Blob,
            File: f.File,
            AbortController: a.AbortController,
            AbortSignal: a.AbortSignal,
        });

        // 09_file.js (above) clobbers URL.createObjectURL/revokeObjectURL as a side effect of
        // loading -- it assigns its own real, op-backed versions, which only accept instances
        // of deno_web's Blob. Reassert mini's original hand-rolled ones (hx-download.js is the
        // only caller) so this keeps working once Blob/File stop being deno_web's below: the
        // op-backed versions' storage/URL registry is never actually used here (nothing
        // resolves a `blob:` URL back into bytes), only their type check would bite.
        const _blobs = new Map();
        URL.createObjectURL = (obj) => {
            const id = `blob:local/${Deno.core.ops.op_crypto_random_uuid()}`;
            _blobs.set(id, obj);
            return id;
        };
        URL.revokeObjectURL = (id) => _blobs.delete(id);
    }

    // Kept off the globals: the page must keep jsdom's FormData/Blob/File/AbortSignal, which
    // deno's classes would otherwise replace and which <form> elements only produce natively.
    {
        const h = Deno.core.loadExtScript("ext:deno_fetch/20_headers.js");
        const rq = Deno.core.loadExtScript("ext:deno_fetch/23_request.js");
        const rs = Deno.core.loadExtScript("ext:deno_fetch/23_response.js");
        const fd = Deno.core.loadExtScript("ext:deno_fetch/21_formdata.js");
        globalThis.__denoFetch = {
            Headers: h.Headers, Request: rq.Request, Response: rs.Response, FormData: fd.FormData,
        };
    }

    // MessagePort — same reason as Blob/AbortSignal above (undici's webidl module-scope type
    // assertion needs the constructor to exist), but never actually constructed here: mini has
    // no worker/postMessage support, so this is a type-only stand-in, not a real port.
    globalThis.MessagePort ??= class MessagePort {};

    // Internal bridge helpers with no per-navigation state (no closure over `win`/`dom`/host
    // state), so they're defined once here instead of being rebuilt on every bootstrap.js
    // restore. Kept on `__mini` rather than as bare globals since they're plumbing, not
    // page-visible API — `__mini` itself may not exist yet this early (element_registry.js/
    // submit.js create it later in the cold pass), hence the `??=`.
    globalThis.__mini ??= {};

    // Resource-mount interceptor's content-type sniffing (bootstrap.js's resourceInterceptor).
    globalThis.__mini.contentTypeFor = function (path) {
        if (path.endsWith(".mjs") || path.endsWith(".js")) return "text/javascript";
        if (path.endsWith(".css")) return "text/css";
        if (path.endsWith(".json")) return "application/json";
        if (path.endsWith(".html")) return "text/html";
        return "application/octet-stream";
    };

    // Shared by fetch()'s own FormData handling (bootstrap.js) and submit.js's POST branch.
    globalThis.__mini.serializeMultipart = function (formData) {
        const boundary = "----HxClientBoundary" + Math.random().toString(36).slice(2, 18);
        let body = "";
        for (const [name, value] of formData) {
            body += `--${boundary}\r\nContent-Disposition: form-data; name="${name}"\r\n\r\n${value}\r\n`;
        }
        body += `--${boundary}--`;
        return { boundary, body };
    };

    // Per-Window jsdom polyfills, installed by bootstrap.js's registerWindowGlobals on every
    // navigation. Each takes the new `win` as a parameter rather than closing over one, so
    // they're reusable across every Window instance instead of being redefined per restore.

    // XPathEvaluator.prototype.createExpression — short-circuit the two XPath shapes htmx
    // actually compiles (#hxOnQuery and hx-live's bind scan, both
    // `.//*[@*[starts-with(name(), "...") or @exact]]`) against patch-jsdom-hxon-index.js's
    // grow-only Set instead of walking the real XPath engine over the whole subtree on every
    // process(). A superset match is safe here (see that file's header); this is the only
    // reader of its Set, kept on globalThis (not per-document -- see that file's header for
    // why). win.XPathEvaluator is a fresh class per Window, so this must be reapplied every
    // navigation like the ANY_TYPE patch in bootstrap.js.
    globalThis.__mini.installXPathShortCircuit = function (win) {
        const HXON_INDEX_KEY = Symbol.for("miniclient.hxOnIndex");
        function hxOnShortCircuit(expr) {
            if (!expr.startsWith(".//*[@*[") || !expr.includes("starts-with(name()")) return null;
            const prefixes = [...expr.matchAll(/starts-with\(name\(\),\s*"([^"]*)"\)/g)].map(
                (m) => m[1],
            );
            const exacts = [...expr.matchAll(/(?:^|[\s([])@([a-z][\w:-]*)/gi)].map((m) => m[1]);
            const matchName = (n) => prefixes.some((p) => n.startsWith(p)) || exacts.includes(n);
            return function evaluate(ctxWrapper) {
                const ctx = globalThis.__jsdomImplForWrapper(ctxWrapper);
                const index = globalThis[HXON_INDEX_KEY];
                const out = [];
                if (index) {
                    for (const el of index) {
                        if (!el.isConnected) {
                            index.delete(el);
                            continue;
                        }
                        if (ctx !== el && !ctx.contains(el)) continue;
                        if (el.getAttributeNames().some(matchName))
                            out.push(globalThis.__jsdomWrapperForImpl(el));
                    }
                }
                let i = 0;
                return { iterateNext: () => out[i++] ?? null };
            };
        }
        const origCreateExpression = win.XPathEvaluator.prototype.createExpression;
        win.XPathEvaluator.prototype.createExpression = function (expression, resolver) {
            const shortCircuit = hxOnShortCircuit(expression);
            if (shortCircuit) return { evaluate: shortCircuit };
            return origCreateExpression.call(this, expression, resolver);
        };
    };

    // jsdom's parser leaves multiple checked radios in one group checked when markup is
    // assigned through innerHTML. Reapply the radio-group invariant after that parse; the
    // checked setter already handles the sibling exclusion.
    globalThis.__mini.installInnerHtmlRadioFix = function (win) {
        const innerHTML = Object.getOwnPropertyDescriptor(win.Element.prototype, "innerHTML");
        const originalSet = innerHTML.set;
        Object.defineProperty(win.Element.prototype, "innerHTML", {
            get: innerHTML.get,
            set(value) {
                originalSet.call(this, value);
                const groups = new Map();
                for (const radio of this.querySelectorAll('input[type="radio"][name]')) {
                    if (!radio.name || !radio.checked) continue;
                    const byName = groups.get(radio.form ?? radio.getRootNode()) ?? new Map();
                    const previous = byName.get(radio.name);
                    if (previous) previous.checked = false;
                    byName.set(radio.name, radio);
                    groups.set(radio.form ?? radio.getRootNode(), byName);
                }
            },
            configurable: true,
        });
    };

    // innerText: jsdom has no layout engine, so it never implements this (needs rendered-box
    // information). styleless-innertext (ported, not reimplemented) approximates it from the
    // DOM tree + computed style instead of real layout. Its own algorithm has no hidden-element
    // exclusion, so a self/ancestor `hidden` attribute or display:none/visibility:hidden check
    // is added here; hidden *descendants* still leak their text, an accepted gap since real
    // layout info doesn't exist to detect them either.
    // DOMParser-created documents have no browsing context; jsdom rejects cross-window style
    // lookup, so detached fragments use textContent instead.
    globalThis.__mini.installInnerText = function (win) {
        function isMiniHidden(el) {
            for (let node = el; node; node = node.parentElement) {
                if (node.hidden) return true;
                const ownerWindow = node.ownerDocument?.defaultView;
                if (!ownerWindow) continue;
                const style = ownerWindow.getComputedStyle(node);
                if (style.display === "none" || style.visibility === "hidden") return true;
            }
            return false;
        }
        Object.defineProperty(win.HTMLElement.prototype, "innerText", {
            get() {
                if (isMiniHidden(this)) return "";
                const ownerWindow = this.ownerDocument?.defaultView;
                if (!ownerWindow) return this.textContent;
                return globalThis.__innerText(this, {
                    getComputedStyle: (el) => ownerWindow.getComputedStyle(el),
                });
            },
            set(value) {
                this.textContent = value;
            },
            configurable: true,
        });
    };

    globalThis.CSS = {
        escape(value) {
            value = String(value);
            if (value.length === 0) return value;
            let out = "";
            for (let i = 0; i < value.length; i++) {
                const c = value.charCodeAt(i);
                if (c === 0) {
                    out += "�";
                    continue;
                }
                if ((c >= 0x0001 && c <= 0x001f) || c === 0x007f) {
                    out += "\\" + c.toString(16) + " ";
                    continue;
                }
                if (i === 0 && c >= 0x0030 && c <= 0x0039) {
                    out += "\\" + c.toString(16) + " ";
                    continue;
                }
                if (i === 1 && c >= 0x0030 && c <= 0x0039 && value.charCodeAt(0) === 0x002d) {
                    out += "\\" + c.toString(16) + " ";
                    continue;
                }
                if (i === 0 && value.length === 1 && c === 0x002d) {
                    out += "\\" + value[i];
                    continue;
                }
                if (
                    c >= 0x0080 ||
                    c === 0x002d ||
                    c === 0x005f ||
                    (c >= 0x0030 && c <= 0x0039) ||
                    (c >= 0x0041 && c <= 0x005a) ||
                    (c >= 0x0061 && c <= 0x007a)
                ) {
                    out += value[i];
                    continue;
                }
                out += "\\" + value[i];
            }
            return out;
        },
    };
})();
