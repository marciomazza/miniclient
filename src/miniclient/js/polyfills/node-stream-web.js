export const ReadableStream = globalThis.ReadableStream;
export const WritableStream = globalThis.WritableStream;
export const TransformStream = globalThis.TransformStream;

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

export default { ReadableStream, WritableStream, TransformStream };
