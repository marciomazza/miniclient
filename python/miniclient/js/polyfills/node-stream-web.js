// The `stream/web` bundle alias. happy-dom's FetchBodyUtility does
// `body instanceof ReadableStream` against this import, so it must resolve to the exact
// global class. pre_globals.js loads deno_web's 06_streams.js and assigns these onto
// globalThis in the cold snapshot pass, before the happy-dom bundle evals.
export const ReadableStream = globalThis.ReadableStream;
export const WritableStream = globalThis.WritableStream;
export const TransformStream = globalThis.TransformStream;

export default { ReadableStream, WritableStream, TransformStream };
