// @ts-check
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
    // @ts-expect-error -- only the parts the bundle reads, not node's ProcessVersions
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
// @ts-expect-error -- no subtle; only random values and UUIDs are provided
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
// @ts-expect-error -- partial stand-in, only the prototype descriptors are read
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
// @ts-expect-error -- a plain timer, not a node Immediate
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

// AbortController/AbortSignal — deno_web's, so the signal a page hands to fetch() passes
// deno_fetch's Request brand check. jsdom's EventTarget#addEventListener accepts this class
// for its `{ signal }` option through a jsdom patch.
{
    const abort = Deno.core.loadExtScript("ext:deno_web/03_abort_signal.js");
    Object.assign(globalThis, {
        AbortController: abort.AbortController,
        AbortSignal: abort.AbortSignal,
    });
}

// Kept off the globals: the page must keep jsdom's FormData/Blob/File/AbortSignal, which
// deno's classes would otherwise replace and which <form> elements only produce natively.
{
    const headers = Deno.core.loadExtScript("ext:deno_fetch/20_headers.js");
    const request = Deno.core.loadExtScript("ext:deno_fetch/23_request.js");
    const response = Deno.core.loadExtScript("ext:deno_fetch/23_response.js");
    const formData = Deno.core.loadExtScript("ext:deno_fetch/21_formdata.js");
    const body = Deno.core.loadExtScript("ext:deno_fetch/22_body.js");
    const file = Deno.core.loadExtScript("ext:deno_web/09_file.js");

    // Loading 09_file.js installs op-backed URL.createObjectURL/revokeObjectURL that only
    // accept deno's Blob, but pages pass jsdom's. Nothing resolves a `blob:` URL back to
    // bytes, so a plain id registry is enough.
    const blobs = new Map();
    URL.createObjectURL = (obj) => {
        const id = `blob:local/${Deno.core.ops.op_crypto_random_uuid()}`;
        blobs.set(id, obj);
        return id;
    };
    URL.revokeObjectURL = (id) => blobs.delete(id);
    globalThis.__denoFetch = {
        Headers: headers.Headers,
        Request: request.Request,
        Response: response.Response,
        FormData: formData.FormData,
        Blob: file.Blob,
        File: file.File,
        // Response internals: `new Response()` cannot build opaque, cors or redirected ones.
        InnerBody: body.InnerBody,
        newInnerResponse: response.newInnerResponse,
        fromInnerResponse: response.fromInnerResponse,
    };
}

// @ts-expect-error -- only CSS.escape is provided
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
