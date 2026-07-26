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
