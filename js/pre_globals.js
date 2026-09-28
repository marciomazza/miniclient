(function () {
    // Real, circular-safe console (replaces deno_core's built-in one, which throws on any
    // circular arg -- htmx passes those). 00_url.js is force-loaded too: 01_console.js
    // lazy-loads it on first use, and only scripts touched here (the snapshot's cold pass)
    // survive into the snapshot's residual lazy table.
    //
    // URL/URLSearchParams — deno_web's WHATWG impl (Servo `url` crate). Assigned here in the
    // cold pass so happy-dom's bundle captures them as the globals when it evals; node-url.js
    // re-exports them for the `url` bundle alias, which happy-dom's URL/URLSearchParams import
    // from. searchParams mutations propagate back to href, and URLSearchParams accepts an
    // iterable (incl. our FormData) as init -- both former patch-happy-dom-url.js jobs.
    {
        const url = Deno.core.loadExtScript("ext:deno_web/00_url.js");
        const { Console } = Deno.core.loadExtScript("ext:deno_web/01_console.js");
        globalThis.console = new Console(Deno.core.print);

        Object.assign(globalThis, { URL: url.URL, URLSearchParams: url.URLSearchParams });
    }

    // TextEncoder/TextDecoder — deno_web's, backed by encoding_rs ops. Force-loaded here (cold
    // snapshot pass) so happy-dom captures the native classes when its bundle evals. webidl
    // must be touched too: only ext scripts loaded in the cold pass survive the snapshot.
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
    // marks/measures/observers. Force-loaded here (cold snapshot pass) so happy-dom's bundle
    // captures the native classes and the singleton when it evals; node-perf-hooks.js re-exports
    // PerformanceObserver/PerformanceEntry from the globals set below.
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

    // crypto — CSPRNG backed by native ops (getrandom + the uuid crate). Must exist before
    // happy-dom is imported: its BrowserWindow does `import { webcrypto } from 'crypto';
    // crypto = webcrypto`, which resolves through node-crypto.js's `globalThis.crypto`
    // forwarding. Only getRandomValues + randomUUID are used anywhere; no crypto.subtle.
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

    // V8 hides the SharedArrayBuffer constructor unless the embedder marks the context as
    // cross-origin isolated (post-Spectre mitigation, normally driven by COOP/COEP headers);
    // this runtime never does that, so the global is simply absent (WeakRef and Atomics are
    // both present, just not this one) -- must exist before jsdom is imported: both jsdom's
    // generated/idl/utils.js and its webidl-conversions dependency read own
    // "byteLength"/"growable" descriptors off `SharedArrayBuffer.prototype` at module top
    // level, and jsdom never constructs one for anything mini actually exercises (no real
    // network resource loading, no native XHR/WebSocket).
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

    // Must exist before happy-dom is imported: its modules do
    // `globalThis.setTimeout.bind(globalThis)` at import time and keep that bound
    // reference forever, so happy-dom's internal timers run on whatever is defined here.
    // win.setTimeout (happy-dom) never fires in this runtime because its internal timer
    // queue is never drained; these replace it entirely.
    {
        let _nextId = 1;
        // The pending op_sleep promise outlives a cancelled timer until `ms` elapses, so the
        // callback lives in these tables (dropped on cancel), never in that promise's closure:
        // a cleared 60s request timeout would otherwise pin its whole page for a minute.
        const _active = {}; // timerId -> { fn, args }
        const _intervals = {}; // intervalId -> { timerId, fn, args }

        const _wait = (ms) => Deno.core.ops.op_sleep(ms || 0);

        globalThis.setTimeout = (fn, ms = 0, ...args) => {
            const id = _nextId++;
            _active[id] = { fn, args };
            _wait(ms).then(() => {
                const timer = _active[id];
                if (timer) {
                    delete _active[id];
                    timer.fn(...timer.args);
                }
            });
            return id;
        };

        globalThis.clearTimeout = (id) => {
            if (id != null) delete _active[id];
        };

        const _scheduleInterval = (intervalId, ms) => {
            const timerId = _nextId++;
            _intervals[intervalId].timerId = timerId;
            _active[timerId] = true;
            _wait(ms).then(() => {
                delete _active[timerId];
                const interval = _intervals[intervalId];
                if (interval) {
                    interval.fn(...interval.args);
                    // The callback may have cleared its own interval.
                    if (_intervals[intervalId]) _scheduleInterval(intervalId, ms);
                }
            });
        };

        globalThis.setInterval = (fn, ms = 0, ...args) => {
            const intervalId = _nextId++;
            _intervals[intervalId] = { timerId: 0, fn, args };
            _scheduleInterval(intervalId, ms);
            return intervalId;
        };

        globalThis.clearInterval = (intervalId) => {
            const interval = _intervals[intervalId];
            delete _intervals[intervalId];
            if (interval) delete _active[interval.timerId];
        };

        // Silently drop all pending timer callbacks — used between tests to prevent
        // stale timers from a previous test firing into the next one's context.
        globalThis.__clearAllTimers = () => {
            for (const k of Object.keys(_active)) delete _active[k];
            for (const k of Object.keys(_intervals)) delete _intervals[k];
        };
    }
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
    // pass) so happy-dom's bundle captures these as the global classes; node-stream-web.js
    // re-exports them for the `stream/web` bundle alias, and FetchBodyUtility's
    // `body instanceof ReadableStream` check needs that to be the exact global.
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
    // patches/jsdom+30.0.1.patch).
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

    // MessagePort — same reason as Blob/AbortSignal above (undici's webidl module-scope type
    // assertion needs the constructor to exist), but never actually constructed here: mini has
    // no worker/postMessage support, so this is a type-only stand-in, not a real port.
    globalThis.MessagePort ??= class MessagePort {};

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
