export let ReadableStream = globalThis.ReadableStream;
export let WritableStream = globalThis.WritableStream;
export let TransformStream = globalThis.TransformStream;

// When this module is baked into the V8 snapshot (see build-happydom-bundle.mjs),
// globalThis.ReadableStream doesn't exist yet at snapshot-build time — the jsrun host only
// provides it once a real Runtime starts. Re-read the globals and (re)apply the patch below
// once that's true; bootstrap.js calls this again on every real Runtime startup.
export function __refreshStreamGlobals() {
    ReadableStream = globalThis.ReadableStream;
    WritableStream = globalThis.WritableStream;
    TransformStream = globalThis.TransformStream;

    // This engine's ReadableStream predates the (newer) async-iterator addition to the
    // spec, but happy-dom's XMLHttpRequest unconditionally does `for await (chunk of
    // response.body)` — polyfill it in terms of the reader API, which is supported.
    if (!ReadableStream.prototype[Symbol.asyncIterator]) {
        ReadableStream.prototype[Symbol.asyncIterator] = async function* () {
            const reader = this.getReader();
            try {
                for (;;) {
                    const { done, value } = await reader.read();
                    if (done) return;
                    yield value;
                }
            } finally {
                reader.releaseLock?.();
            }
        };
    }

    // This engine's getReader() returns a plain object (read/cancel as own properties,
    // no shared reader prototype to patch once), and it has no releaseLock at all —
    // there's no multi-reader lock to release here, so callers that follow the spec and
    // call reader.releaseLock() unconditionally (e.g. htmx's hx-multipart extension) throw.
    if (!ReadableStream.prototype.getReader.__patchedReleaseLock) {
        const _origGetReader = ReadableStream.prototype.getReader;
        ReadableStream.prototype.getReader = function (...args) {
            const reader = _origGetReader.apply(this, args);
            if (reader && typeof reader.releaseLock !== "function") reader.releaseLock = () => {};
            return reader;
        };
        ReadableStream.prototype.getReader.__patchedReleaseLock = true;
    }

    // Two related gaps in this engine's ReadableStream, both hit by hx-multipart.js's
    // pull-driven body parser:
    //
    // 1. A controller handed to start()/pull() has no `desiredSize` at all (not even
    //    `undefined` shows up in a property listing) instead of the spec's backpressure
    //    number, so `(controller.desiredSize ?? 0) > 0` reads as permanently false.
    // 2. More fundamentally, a start()-only (no `pull`) source resolves the *next*
    //    read() as EOF as soon as the queue drains, instead of waiting for a later
    //    enqueue()/close() call (e.g. from a setTimeout callback) — real engines keep
    //    that read pending. Any stream fed asynchronously after construction loses
    //    every chunk enqueued after the first read.
    //
    // Fix both by wrapping the constructor: track a per-stream pending-chunk counter for
    // desiredSize, and always install a pull() that stays pending until the next
    // enqueue/close/error, so the engine never mistakes "nothing queued right now" for
    // "nothing more is coming". Byte-stream controllers (`type: "bytes"`) are left alone —
    // nothing in this codebase constructs one.
    if (!ReadableStream.__patchedBackpressure) {
        const _OrigReadableStream = ReadableStream;
        const _controllerGetterByStream = new WeakMap();

        const PatchedReadableStream = function (source, strategy) {
            let wrapped = source;
            let controllerRef = null;
            if (source && typeof source === "object" && source.type !== "bytes") {
                wrapped = { ...source };
                let pending = 0;
                let closed = false;
                let waiters = [];
                const notify = () => {
                    const ws = waiters;
                    waiters = [];
                    ws.forEach((fn) => fn());
                };

                if (typeof source.start === "function") {
                    const origStart = source.start.bind(source);
                    wrapped.start = (controller) => {
                        controllerRef = controller;
                        const origEnqueue = controller.enqueue.bind(controller);
                        const origClose = controller.close.bind(controller);
                        const origError = controller.error.bind(controller);
                        controller.enqueue = (chunk) => {
                            pending++;
                            const r = origEnqueue(chunk);
                            notify();
                            return r;
                        };
                        controller.close = (...a) => {
                            closed = true;
                            const r = origClose(...a);
                            notify();
                            return r;
                        };
                        controller.error = (...a) => {
                            closed = true;
                            const r = origError(...a);
                            notify();
                            return r;
                        };
                        Object.defineProperty(controller, "desiredSize", {
                            get: () => (closed ? null : pending > 0 ? 0 : 1),
                            configurable: true,
                        });
                        controller.__markRead = () => {
                            if (pending > 0) pending--;
                        };
                        return origStart(controller);
                    };
                }

                const origPull = typeof source.pull === "function" ? source.pull.bind(source) : null;
                wrapped.pull = async (controller) => {
                    if (origPull) await origPull(controller);
                    if (closed) return;
                    return new Promise((resolve) => waiters.push(resolve));
                };
            }
            const stream = new _OrigReadableStream(wrapped, strategy);
            _controllerGetterByStream.set(stream, () => controllerRef);
            return stream;
        };
        PatchedReadableStream.prototype = _OrigReadableStream.prototype;
        PatchedReadableStream.__patchedBackpressure = true;
        globalThis.ReadableStream = PatchedReadableStream;
        ReadableStream = PatchedReadableStream;

        // desiredSize (above) only flips back to "wants more" once a reader actually
        // consumes the pending chunk.
        const _origGetReader = ReadableStream.prototype.getReader;
        ReadableStream.prototype.getReader = function (...args) {
            const reader = _origGetReader.apply(this, args);
            const getController = _controllerGetterByStream.get(this);
            if (reader && getController && typeof reader.read === "function") {
                const origRead = reader.read.bind(reader);
                reader.read = async function () {
                    const result = await origRead();
                    if (!result.done) {
                        const c = getController();
                        if (c && c.__markRead) c.__markRead();
                    }
                    return result;
                };
            }
            return reader;
        };
    }
}

if (typeof globalThis.ReadableStream !== "undefined") __refreshStreamGlobals();

export default {
    get ReadableStream() {
        return ReadableStream;
    },
    get WritableStream() {
        return WritableStream;
    },
    get TransformStream() {
        return TransformStream;
    },
};
