// jsrun has no native `stream/web` implementation — it only installs its own admittedly
// "minimal polyfill" (see jsrun's src/runtime/ops.rs) when nothing else provides
// `ReadableStream`. That fallback lacks backpressure (`desiredSize`) and resolves reads as
// EOF prematurely for sources that fill their queue asynchronously (e.g. via setTimeout
// inside start()) — both hit by htmx's hx-multipart.js body parser. happy-dom itself doesn't
// implement streams either; it just re-exports whatever `stream/web` the host (normally
// Node.js) provides. So we bring a real, spec-compliant implementation instead.
import {
    ReadableStream as PolyfillReadableStream,
    WritableStream as PolyfillWritableStream,
    TransformStream as PolyfillTransformStream,
} from "web-streams-polyfill";

export let ReadableStream = PolyfillReadableStream;
export let WritableStream = PolyfillWritableStream;
export let TransformStream = PolyfillTransformStream;

// bootstrap.js calls this on every real Runtime startup, after the registration-copy step
// that would otherwise let jsrun's own (buggy) globalThis.ReadableStream stand — reassert
// ours so it wins.
export function __refreshStreamGlobals() {
    globalThis.ReadableStream = ReadableStream;
    globalThis.WritableStream = WritableStream;
    globalThis.TransformStream = TransformStream;
}

__refreshStreamGlobals();

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
